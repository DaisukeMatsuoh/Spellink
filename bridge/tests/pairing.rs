//! Pairing and the command line's door, over the wire (Go: pairing_test.go).

mod common;

use common::*;
use serde_json::{Value, json};
use spellink_bridge::devices;
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn the_pairing_ceremony_end_to_end_over_the_wire() {
    let e = PairEnv::new().await;
    let (id, b) = e.add_device("Nomad");
    let mut d = e.dial(&id, &b).await;
    d.send(json!({"type": "pairRequest", "id": id, "gen": 1, "code": "12345678"})).await;
    e.wait_waiting(&id, 1).await;

    let (status, body) = e.admin("POST", "/admin/pair", Some(json!({"code": "12345678"}))).await;
    assert!(status == 200 && body.contains("Nomad"), "confirm: {status} {body}");
    let g = d.recv().await.unwrap();
    let key = g["key"].as_str().unwrap_or("");
    assert!(g["type"] == "pairGrant" && devices::valid_secret(key) && g["endpoints"].as_array().unwrap().len() == 1, "grant = {g}");
    d.send(json!({"type": "pairAck", "id": id, "gen": 1})).await;
    assert!(d.recv().await.is_none(), "the channel must end after the ack");

    // from now on only K works, and B no longer opens anything
    let mut k = e.dial(&id, key).await;
    assert_eq!(k.recv().await.unwrap()["type"], "welcome", "a session with K must start with welcome");
    assert!(Device::try_open(&format!("{}?d={id}", e.base), &b).await.is_err(), "B must be refused once K has been acknowledged");
}

#[tokio::test]
async fn a_connection_with_k_counts_as_the_ack_over_the_wire() {
    let e = PairEnv::new().await;
    let (id, b) = e.add_device("Nomad");
    let mut d = e.dial(&id, &b).await;
    d.send(json!({"type": "pairRequest", "id": id, "gen": 1, "code": "12345678"})).await;
    e.wait_waiting(&id, 1).await;
    e.admin("POST", "/admin/pair", Some(json!({"code": "12345678"}))).await;
    let key = d.recv().await.unwrap()["key"].as_str().unwrap().to_string();
    // the device stored K but its ack never arrived: it connects with K instead
    assert_eq!(e.dial(&id, &key).await.recv().await.unwrap()["type"], "welcome");
    assert_eq!(e.reg.list()[0].status, "active");
}

#[tokio::test]
async fn the_pairing_channel_allows_nothing_but_pairing() {
    let e = PairEnv::new().await;
    let (id, b) = e.add_device("Nomad");
    let mut d = e.dial(&id, &b).await;
    d.send(json!({"type": "ask", "id": "q", "question": "hi", "lang": "ja", "layout": layout()})).await;
    let m = d.recv().await.unwrap();
    assert!(m["type"] == "error" && m["code"] == "bad_request", "{m}");
    assert!(d.recv().await.is_none(), "the channel must be closed after a forbidden frame");
}

#[tokio::test]
async fn pairing_request_rules() {
    let e = PairEnv::new().await;
    let (id, b) = e.add_device("Nomad");
    let (other, _) = e.add_device("Other");

    let mut d = e.dial(&id, &b).await; // a request for another device
    d.send(json!({"type": "pairRequest", "id": other, "gen": 1, "code": "11111111"})).await;
    assert_eq!(d.recv().await.unwrap()["code"], "bad_request", "another device");
    let mut d = e.dial(&id, &b).await; // two requests on one channel
    d.send(json!({"type": "pairRequest", "id": id, "gen": 1, "code": "11111111"})).await;
    e.wait_waiting(&id, 1).await;
    d.send(json!({"type": "pairRequest", "id": id, "gen": 1, "code": "22222222"})).await;
    assert_eq!(d.recv().await.unwrap()["code"], "bad_request", "second request");
    let mut d = e.dial(&id, &b).await; // a malformed code
    d.send(json!({"type": "pairRequest", "id": id, "gen": 1, "code": "12"})).await;
    let m = d.recv().await.unwrap();
    assert!(m["type"] == "pairDenied" && m["reason"] == "unknown_device", "bad code: {m}");
    let mut d = e.dial(&id, &b).await; // a stale package (old generation)
    d.send(json!({"type": "pairRequest", "id": id, "gen": 7, "code": "12345678"})).await;
    assert_eq!(d.recv().await.unwrap()["type"], "pairDenied", "old generation");
    let mut d = e.dial(&id, &b).await; // an ack for another device
    d.send(json!({"type": "pairAck", "id": other, "gen": 1})).await;
    assert_eq!(d.recv().await.unwrap()["code"], "bad_request", "ack for another device");
}

#[tokio::test]
async fn the_same_code_from_two_channels_denies_both() {
    let e = PairEnv::new().await;
    let (id, b) = e.add_device("Nomad");
    let mut first = e.dial(&id, &b).await;
    first.send(json!({"type": "pairRequest", "id": id, "gen": 1, "code": "12345678"})).await;
    e.wait_waiting(&id, 1).await;
    let mut second = e.dial(&id, &b).await;
    second.send(json!({"type": "pairRequest", "id": id, "gen": 1, "code": "12345678"})).await;
    for (i, d) in [&mut first, &mut second].into_iter().enumerate() {
        let m = d.recv().await.unwrap();
        assert!(m["type"] == "pairDenied" && m["reason"] == "code_in_use", "channel {i}: {m}");
    }
    let (status, _) = e.admin("POST", "/admin/pair", Some(json!({"code": "12345678"}))).await;
    assert_eq!(status, 404, "nothing may be left to confirm");
}

#[tokio::test]
async fn a_channel_that_never_requests_is_closed() {
    let e = PairEnv::with(|d, _| d.pair_request_wait = Duration::from_millis(100)).await;
    let (id, b) = e.add_device("Nomad");
    assert!(e.dial(&id, &b).await.recv().await.is_none(), "a silent pairing channel must be closed");
}

#[tokio::test]
async fn a_pairing_channel_has_a_maximum_life() {
    let e = PairEnv::with(|d, _| d.pairing_max = Duration::from_millis(150)).await;
    let (id, b) = e.add_device("Nomad");
    let mut d = e.dial(&id, &b).await;
    d.send(json!({"type": "pairRequest", "id": id, "gen": 1, "code": "12345678"})).await;
    assert!(d.recv().await.is_none(), "the channel must end when its life is over");
}

#[tokio::test]
async fn device_connections_are_refused_unless_they_are_properly_encrypted() {
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::Message;
    let e = PairEnv::new().await;
    let (id, b) = e.add_device("Nomad");
    // each attempt must end with the bridge closing the session
    async fn ends(url: &str, first: Message) -> bool {
        let mut c = Client::connect(url).await;
        c.ws.send(first).await.unwrap();
        c.recv().await.is_none()
    }
    assert!(ends(&format!("{}?d={id}", e.base), Message::Text(r#"{"type":"hello","token":"secret"}"#.into())).await, "plaintext hello");
    assert!(Device::try_open(&format!("{}?d=ffffffffffffffff", e.base), &b).await.is_err(), "unknown device");
    assert!(Device::try_open(&format!("{}?d={id}", e.base), "not the secret").await.is_err(), "wrong secret");
    assert!(ends(&format!("{}?d={id}", e.base), Message::Binary(b"garbage".to_vec().into())).await, "no secret at all");
    // a ledger-less bridge refuses every device
    let plain = start(ai(|_, _, _, _| async { Ok(String::new()) })).await;
    assert!(Device::try_open(&format!("{plain}?d={id}"), &b).await.is_err(), "no ledger");
}

#[tokio::test]
async fn revoking_a_device_cuts_off_its_live_session() {
    let e = PairEnv::new().await;
    let (id, k) = e.pair_fully("Nomad", "12345678").await;
    let mut live = e.dial(&id, &k).await;
    assert_eq!(live.recv().await.unwrap()["type"], "welcome");
    let (status, _) = e.admin("POST", &format!("/admin/devices/{id}/revoke"), None).await;
    assert_eq!(status, 204, "revoke");
    assert!(live.recv().await.is_none(), "the live session of a revoked device must end");
    assert!(Device::try_open(&format!("{}?d={id}", e.base), &k).await.is_err(), "a revoked device's K must not open a session");
}

// ---- the command line's door ----

#[tokio::test]
async fn admin_refuses_what_is_not_local_and_authenticated() {
    let e = PairEnv::new().await;
    let get = |token: Option<&str>, extra: Option<(&str, &str)>| {
        let mut h: Vec<(String, String)> = vec![];
        if let Some(t) = token {
            h.push(("Authorization".into(), format!("Bearer {t}")));
        }
        if let Some((k, v)) = extra {
            h.push((k.into(), v.into()));
        }
        let (http, h2) = (e.http.clone(), h);
        async move {
            let hs: Vec<(&str, &str)> = h2.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
            http_request(&http, "GET", "/admin/devices", &hs, None).await.0
        }
    };
    assert_eq!(get(None, None).await, 401, "no token");
    assert_eq!(get(Some("wrong"), None).await, 401, "wrong token");
    assert_eq!(get(Some(ADMIN_TOKEN), Some(("Cf-Connecting-Ip", "203.0.113.9"))).await, 403, "through the tunnel");
    assert_eq!(get(Some(ADMIN_TOKEN), Some(("X-Forwarded-For", "203.0.113.9"))).await, 403, "through a proxy");
    assert_eq!(get(Some(ADMIN_TOKEN), None).await, 200, "good request");
    // a bridge configured with no token must refuse everything, not accept the empty one
    let empty = PairEnv::with(|_, a| a.as_mut().unwrap().token = String::new()).await;
    assert_eq!(http_request(&empty.http, "GET", "/admin/devices", &[], None).await.0, 401, "empty configured token");
    assert_eq!(http_request(&empty.http, "GET", "/admin/devices", &[("Authorization", "Bearer ")], None).await.0, 401, "an empty bearer against an empty token");
}

#[tokio::test]
async fn admin_device_lifecycle() {
    let e = PairEnv::new().await;
    let (status, body) = e.admin("POST", "/admin/devices", Some(json!({"name": "Nomad"}))).await;
    let created: Value = serde_json::from_str(&body).unwrap_or_default();
    let id = created["id"].as_str().unwrap_or("").to_string();
    assert!(status == 201 && devices::valid_id(&id) && created["name"] == "Nomad", "add: {status} {body}");
    assert_eq!(e.admin("POST", "/admin/devices", Some(json!({"name": "Nomad"}))).await.0, 409, "duplicate");
    assert_eq!(e.admin("POST", "/admin/devices", Some(json!({"name": ""}))).await.0, 400, "bad name");
    assert_eq!(e.admin("POST", "/admin/devices", Some(json!("not an object"))).await.0, 400, "bad body");
    let (status, body) = e.admin("GET", "/admin/devices", None).await;
    let list: Vec<Value> = serde_json::from_str(&body).unwrap();
    assert!(status == 200 && list.len() == 1 && list[0]["status"] == "issued" && list[0]["expires"].is_string(), "list: {status} {body}");
    assert!(!body.contains("\"b\"") && !body.contains("\"k\""), "the list must never carry a secret");
    let (status, body) = e.admin("GET", &format!("/admin/devices/{id}/package"), None).await;
    assert!(status == 200 && body.starts_with("package for Nomad gen 1 B="), "package: {status} {body}");
    assert_eq!(e.admin("POST", &format!("/admin/devices/{id}/reissue"), None).await.0, 204, "reissue");
    assert_eq!(e.admin("POST", &format!("/admin/devices/{id}/forget"), None).await.0, 409, "forget while valid");
    assert_eq!(e.admin("POST", &format!("/admin/devices/{id}/revoke"), None).await.0, 204, "revoke");
    assert_eq!(e.admin("POST", &format!("/admin/devices/{id}/revoke"), None).await.0, 409, "revoke twice");
    assert_eq!(e.admin("POST", &format!("/admin/devices/{id}/reissue"), None).await.0, 409, "reissue a revoked one");
    assert_eq!(e.admin("GET", &format!("/admin/devices/{id}/package"), None).await.0, 409, "package of a revoked one");
    assert_eq!(e.admin("POST", &format!("/admin/devices/{id}/forget"), None).await.0, 204, "forget");
    for p in ["revoke", "reissue", "forget"] {
        assert_eq!(e.admin("POST", &format!("/admin/devices/ffffffffffffffff/{p}"), None).await.0, 404, "{p} of an unknown device");
    }
    assert_eq!(e.admin("GET", "/admin/devices/ffffffffffffffff/package", None).await.0, 404, "package of an unknown device");
}

#[tokio::test]
async fn admin_pair_and_its_errors() {
    let e = PairEnv::new().await;
    assert_eq!(e.admin("POST", "/admin/pair", Some(json!({"code": "1"}))).await.0, 400, "short code");
    assert_eq!(e.admin("POST", "/admin/pair", Some(json!({"code": "12345678"}))).await.0, 404, "nobody waiting");
    assert_eq!(e.admin("POST", "/admin/pair", Some(json!(5))).await.0, 400, "bad body");
    let (id, b) = e.add_device("Nomad");
    let mut d = e.dial(&id, &b).await;
    d.send(json!({"type": "pairRequest", "id": id, "gen": 1, "code": "12345678"})).await;
    e.wait_waiting(&id, 1).await;
    let (status, body) = e.admin("POST", "/admin/pair", Some(json!({"code": "00000000"}))).await;
    assert!(status == 403 && !body.contains("Nomad"), "wrong code: {status} {body} (must not name the waiting device)");
}

#[tokio::test]
async fn admin_reports_an_unwritable_ledger_and_a_broken_package() {
    let e = PairEnv::new().await;
    *e.fail.lock().unwrap() = Some("disk full".into());
    assert_eq!(e.admin("POST", "/admin/devices", Some(json!({"name": "Nomad"}))).await.0, 500, "unwritable ledger");
    let broken = PairEnv::with(|_, a| a.as_mut().unwrap().package = Arc::new(|_| Err("no base package".into()))).await;
    let (bid, _) = broken.add_device("Nomad");
    assert_eq!(broken.admin("GET", &format!("/admin/devices/{bid}/package"), None).await.0, 500, "broken package");
}

#[tokio::test]
async fn admin_is_off_unless_configured() {
    let e = PairEnv::with(|_, a| *a = None).await;
    assert_eq!(http_request(&e.http, "GET", "/admin/devices", &[], None).await.0, 404);
}

#[tokio::test]
async fn the_legacy_shared_token_still_works() {
    let e = PairEnv::new().await;
    let got = converse(&e.base, vec![hello()], |m| m["type"] == "welcome").await;
    assert_eq!(types(&got), "welcome");
}

#[tokio::test]
async fn an_ack_for_the_wrong_generation_does_not_activate_the_device() {
    let e = PairEnv::new().await;
    let (id, b) = e.add_device("Nomad");
    let mut d = e.dial(&id, &b).await;
    d.send(json!({"type": "pairRequest", "id": id, "gen": 1, "code": "12345678"})).await;
    e.wait_waiting(&id, 1).await;
    e.admin("POST", "/admin/pair", Some(json!({"code": "12345678"}))).await;
    assert_eq!(d.recv().await.unwrap()["type"], "pairGrant");
    d.send(json!({"type": "pairAck", "id": id, "gen": 99})).await;
    assert!(d.recv().await.is_none(), "the channel ends after an ack, valid or not");
    assert_eq!(e.reg.list()[0].status, "granting", "only a matching ack may activate");
}

#[tokio::test]
async fn a_grant_without_endpoints_sends_an_empty_list() {
    let dir = tempfile::tempdir().unwrap();
    let sessions = Arc::new(spellink_bridge::server::session::Sessions::default());
    let (reg, _) = devices::Registry::new(devices::Config::default(), Box::new(devices::FileStore::new(dir.path().join("d.json"))), devices::Hooks::default(), devices::os_rand()).unwrap();
    let r2 = reg.clone();
    let e = PairEnv::with(move |d, a| {
        d.devices = Some(r2.clone());
        d.sessions = sessions;
        a.as_mut().unwrap().devices = r2;
    })
    .await;
    let id = reg.add("Nomad").unwrap();
    let pk = reg.repack(&id).unwrap();
    let mut d = e.dial(&id, &pk.b).await;
    d.send(json!({"type": "pairRequest", "id": id, "gen": 1, "code": "12345678"})).await;
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while !reg.list().iter().any(|i| i.waiting == 1) && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    reg.confirm("12345678").unwrap();
    let g = d.recv().await.unwrap();
    assert!(g["type"] == "pairGrant" && g["endpoints"].as_array().is_some_and(|a| a.is_empty()), "grant = {g}");
}

#[tokio::test]
async fn admin_pair_with_the_devices_id_only_considers_that_device() {
    let e = PairEnv::new().await;
    let ((one, b1), (two, b2)) = (e.add_device("one"), e.add_device("two"));
    let (mut d1, mut d2) = (e.dial(&one, &b1).await, e.dial(&two, &b2).await);
    d1.send(json!({"type": "pairRequest", "id": one, "gen": 1, "code": "11111111"})).await;
    d2.send(json!({"type": "pairRequest", "id": two, "gen": 1, "code": "22222222"})).await;
    e.wait_waiting(&one, 1).await;
    e.wait_waiting(&two, 1).await;
    assert_eq!(e.admin("POST", "/admin/pair", Some(json!({"id": one, "code": "22222222"}))).await.0, 403, "the code of another device");
    let (status, body) = e.admin("POST", "/admin/pair", Some(json!({"id": two, "code": "22222222"}))).await;
    assert!(status == 200 && body.contains("two"), "the right id and code: {status} {body}");
    assert_eq!(d2.recv().await.unwrap()["type"], "pairGrant", "the device was granted");
    assert_eq!(e.admin("POST", "/admin/pair", Some(json!({"id": "ffffffffffffffff", "code": "11111111"}))).await.0, 404, "an unknown device");
}

// ---- what a request may carry ----

#[tokio::test]
async fn a_request_that_still_carries_a_banner_is_answered_like_any_other_and_the_first_frame_is_the_grant() {
    // an earlier device asked for a phrase in handwriting; the bridge no longer writes one, and the
    // extra fields change nothing (the first frame the device gets is the grant)
    let e = PairEnv::new().await;
    let (id, b) = e.add_device("Nomad");
    let mut d = e.dial(&id, &b).await;
    d.send(json!({"type": "pairRequest", "id": id, "gen": 1, "code": "12345678", "banner": "ああ", "layout": {"x": 0, "y": 0, "width": 600, "pageHeight": 400, "bottomMargin": 0, "charSize": 40}})).await;
    e.wait_waiting(&id, 1).await;
    e.admin("POST", "/admin/pair", Some(json!({"code": "12345678"}))).await;
    assert_eq!(d.recv().await.unwrap()["type"], "pairGrant");
}

#[tokio::test]
async fn a_denied_request_is_answered_with_the_refusal_and_nothing_else() {
    let e = PairEnv::new().await;
    let (id, b) = e.add_device("Nomad");
    let mut d = e.dial(&id, &b).await;
    d.send(json!({"type": "pairRequest", "id": id, "gen": 99, "code": "12345678"})).await;
    assert_eq!(d.recv().await.unwrap()["type"], "pairDenied");
}

#[tokio::test]
async fn the_local_doors_refuse_a_foreign_host_even_with_the_right_token() {
    let e = PairEnv::new().await;
    let auth = format!("Bearer {ADMIN_TOKEN}");
    // a page that re-pointed its own name at 127.0.0.1
    let (status, _) = http_request(&e.http, "GET", "/admin/devices", &[("Host", "evil.example:8765"), ("Authorization", &auth)], None).await;
    assert_eq!(status, 403, "/admin/devices with a foreign Host");
    for host in ["localhost", "127.0.0.1"] {
        let (status, _) = http_request(&e.http, "GET", "/admin/devices", &[("Host", host), ("Authorization", &auth)], None).await;
        assert_eq!(status, 200, "Host {host}");
    }
}

#[tokio::test]
async fn admin_status_counts_devices_and_passes_on_what_the_bridge_says_about_itself() {
    use spellink_bridge::server::admin::{ProviderInfo, StatusInfo};
    let e = PairEnv::with(|_, a| {
        a.as_mut().unwrap().status = Some(Arc::new(|| StatusInfo {
            version: "1.2.3".into(),
            name: "Rock".into(),
            relay: true,
            addresses: vec!["wss://x/v1".into()],
            provider: ProviderInfo { name: "Claude Code".into(), found: true, path: "/usr/bin/claude".into() },
        }));
    })
    .await;
    e.pair_fully("active", "11111111").await;
    let (id, b) = e.add_device("waiting");
    let mut d = e.dial(&id, &b).await;
    d.send(json!({"type": "pairRequest", "id": id, "gen": 1, "code": "22222222"})).await;
    e.wait_waiting(&id, 1).await;
    e.add_device("issued");
    let (status, body) = e.admin("GET", "/admin/status", None).await;
    let got: Value = serde_json::from_str(&body).unwrap_or_else(|_| panic!("{status} {body}"));
    assert_eq!(status, 200);
    assert!(
        got["version"] == "1.2.3" && got["name"] == "Rock" && got["relay"] == true && got["provider"]["found"] == true && got["provider"]["path"] == "/usr/bin/claude" && got["addresses"].as_array().unwrap().len() == 1,
        "{got}"
    );
    assert!(got["devices"]["total"] == 3 && got["devices"]["active"] == 1 && got["devices"]["waiting"] == 1, "{}", got["devices"]);
}

#[tokio::test]
async fn admin_status_without_a_status_function_still_answers() {
    let e = PairEnv::new().await;
    let (status, body) = e.admin("GET", "/admin/status", None).await;
    assert!(status == 200 && body.contains("\"addresses\":[]") && body.contains("\"total\":0"), "{status} {body}");
}

#[test]
fn the_sessions_table() {
    // add / remove / close with nothing registered are harmless; the real use is in the revoke test
    let t = spellink_bridge::server::session::Sessions::default();
    t.close_device("nobody"); // nothing to close
    assert_eq!(t.count("a"), 0);
}

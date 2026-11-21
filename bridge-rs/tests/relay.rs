//! The relay's behaviour, through its real entry points: WebSocket connections to a listening
//! server (the Go suite's relay_test.go and its limits, idle and bandwidth tests).

use futures_util::{SinkExt, StreamExt};
use spellink_bridge::relay::*;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::protocol::Message;

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Started {
    addr: SocketAddr,
    log: Arc<Mutex<Vec<String>>>,
}

async fn start(cfg: Config) -> Started {
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = log.clone();
    let relay = Relay::new(cfg, Arc::new(move |s| sink.lock().unwrap().push(s.to_string())));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = relay.router().into_make_service_with_connect_info::<SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Started { addr, log }
}

async fn dial(addr: SocketAddr, path: &str, query: &str) -> Ws {
    let url = format!("ws://{addr}{path}?{query}");
    tokio_tungstenite::connect_async(url).await.expect("connect").0
}

async fn next(ws: &mut Ws) -> Option<Message> {
    match tokio::time::timeout(Duration::from_secs(10), ws.next()).await.expect("timed out") {
        Some(Ok(m)) => Some(m),
        _ => None,
    }
}

fn close_code(m: Option<Message>) -> Option<u16> {
    match m {
        Some(Message::Close(Some(f))) => Some(f.code.into()),
        _ => None,
    }
}

/// Opens a relayed session and returns the plugin's and the bridge's ends (and the control socket).
async fn through(cfg: Config) -> (Ws, Ws, Ws) {
    let s = start(cfg).await;
    let room = room_id("secret");
    let mut ctrl = dial(s.addr, BRIDGE_PATH, &format!("room={room}")).await;
    let plugin = dial(s.addr, CLIENT_PATH, &format!("room={room}")).await;
    let m = next(&mut ctrl).await.expect("session frame");
    let v: serde_json::Value = serde_json::from_str(m.to_text().unwrap()).unwrap();
    assert_eq!(v["type"], "session");
    let bridge = dial(s.addr, ACCEPT_PATH, &format!("room={room}&session={}", v["id"].as_str().unwrap())).await;
    (plugin, bridge, ctrl)
}

#[test]
fn the_room_id_hides_the_token() {
    let id = room_id("secret");
    assert!(id.len() == 32 && !id.contains("secret") && id != room_id("other"));
    // the same derivation as the Go relay (internal/relay): sha256("spellink-room-v1:secret")[:16]
    assert_eq!(id, "18e60525e946f6e1251652d1dacf8bb6");
}

#[tokio::test]
async fn a_plugin_before_the_bridge_is_up_is_told_the_bridge_is_offline() {
    let s = start(Config::default()).await;
    let mut c = dial(s.addr, CLIENT_PATH, &format!("room={}", room_id("secret"))).await;
    assert_eq!(close_code(next(&mut c).await), Some(STATUS_BRIDGE_OFFLINE));
}

#[tokio::test]
async fn frames_pass_unchanged_both_ways_whatever_their_type() {
    let (mut plugin, mut bridge, _ctrl) = through(Config::default()).await;
    plugin.send(Message::Text("hello".into())).await.unwrap();
    assert_eq!(next(&mut bridge).await, Some(Message::Text("hello".into())));
    bridge.send(Message::Binary(vec![1, 2, 3].into())).await.unwrap();
    assert_eq!(next(&mut plugin).await, Some(Message::Binary(vec![1, 2, 3].into())));
}

#[tokio::test]
async fn the_relay_refuses_new_rooms_at_its_limit_but_not_a_room_that_exists() {
    let s = start(Config { max_rooms: 2, ..Config::default() }).await;
    let url = |tok: &str| format!("room={}", room_id(tok));
    let _a = dial(s.addr, BRIDGE_PATH, &url("a")).await;
    let _b = dial(s.addr, BRIDGE_PATH, &url("b")).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut third = dial(s.addr, BRIDGE_PATH, &url("c")).await;
    assert_eq!(close_code(next(&mut third).await), Some(STATUS_RELAY_FULL), "a third room is refused");
    let mut again = dial(s.addr, BRIDGE_PATH, &url("a")).await;
    let got = tokio::time::timeout(Duration::from_millis(300), again.next()).await;
    if let Ok(Some(Ok(m))) = got {
        assert_ne!(close_code(Some(m)), Some(STATUS_RELAY_FULL), "an existing room must not be refused");
    }
}

#[tokio::test]
async fn a_newer_control_connection_for_a_room_replaces_the_older() {
    let s = start(Config::default()).await;
    let q = format!("room={}", room_id("x"));
    let mut old = dial(s.addr, BRIDGE_PATH, &q).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let mut newer = dial(s.addr, BRIDGE_PATH, &q).await;
    assert_eq!(close_code(next(&mut old).await), Some(1001));
    // the newer one is the room's bridge: a plugin gets a session frame on it
    let _plugin = dial(s.addr, CLIENT_PATH, &q).await;
    let m = next(&mut newer).await.unwrap();
    assert!(m.to_text().unwrap().contains("\"session\""));
}

#[tokio::test]
async fn a_room_has_a_limited_number_of_sessions() {
    let s = start(Config::default()).await;
    let q = format!("room={}", room_id("x"));
    let _ctrl = dial(s.addr, BRIDGE_PATH, &q).await;
    let mut held = Vec::new();
    for _ in 0..4 {
        held.push(dial(s.addr, CLIENT_PATH, &q).await);
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut fifth = dial(s.addr, CLIENT_PATH, &q).await;
    assert_eq!(close_code(next(&mut fifth).await), Some(STATUS_ROOM_BUSY));
}

#[tokio::test]
async fn an_unknown_session_is_refused_at_accept() {
    let s = start(Config::default()).await;
    let mut c = dial(s.addr, ACCEPT_PATH, &format!("room={}&session=nope", room_id("x"))).await;
    assert_eq!(close_code(next(&mut c).await), Some(1008));
}

async fn http_status(addr: SocketAddr, path: &str, extra: &str) -> u16 {
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    // not a WebSocket upgrade: the answer is only about admission
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n{extra}\r\n").as_bytes()).await.unwrap();
    let mut buf = String::new();
    s.read_to_string(&mut buf).await.unwrap();
    buf.split(' ').nth(1).unwrap().parse().unwrap()
}

#[tokio::test]
async fn one_address_cannot_use_the_relay_up() {
    // the proxy header means nothing unless the operator trusts the proxy that set it
    let s = start(Config { clients_per_min: 3, ..Config::default() }).await;
    let path = format!("{CLIENT_PATH}?room={}", room_id("x"));
    for i in 1..=3 {
        assert_ne!(http_status(s.addr, &path, "").await, 429, "attempt {i} is within the limit");
    }
    assert_eq!(http_status(s.addr, &path, "").await, 429, "the fourth attempt from one address");
    assert_eq!(http_status(s.addr, &path, "Cf-Connecting-Ip: 203.0.113.9\r\n").await, 429, "an untrusted header buys nothing");

    let s = start(Config { clients_per_min: 3, trust_proxy: true, ..Config::default() }).await;
    for _ in 0..4 {
        http_status(s.addr, &path, "").await;
    }
    assert_ne!(http_status(s.addr, &path, "Cf-Connecting-Ip: 203.0.113.9\r\n").await, 429, "behind a trusted proxy each address has its own");
}

#[tokio::test]
async fn a_bad_room_is_a_bad_request() {
    let s = start(Config::default()).await;
    assert_eq!(http_status(s.addr, &format!("{CLIENT_PATH}?room=nothex"), "").await, 400);
    assert_eq!(http_status(s.addr, &format!("{CLIENT_PATH}?room={}", "A".repeat(32)), "").await, 400);
}

#[tokio::test]
async fn health_counts_the_rooms() {
    let s = start(Config::default()).await;
    let _c = dial(s.addr, BRIDGE_PATH, &format!("room={}", room_id("x"))).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let mut st = tokio::net::TcpStream::connect(s.addr).await.unwrap();
    st.write_all(b"GET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").await.unwrap();
    let mut buf = String::new();
    st.read_to_string(&mut buf).await.unwrap();
    assert!(buf.contains("{\"ok\":true,\"bridges\":1}"), "{buf}");
}

#[test]
fn the_limiter_forgets_quiet_addresses_and_starts_a_new_window() {
    let l = Limiter::new(2, Duration::from_secs(60));
    let t0 = Instant::now();
    assert!(l.allow("a", t0) && l.allow("a", t0) && !l.allow("a", t0), "two allowed, the third refused");
    assert!(l.allow("b", t0), "another address has its own allowance");
    assert!(l.allow("a", t0 + Duration::from_secs(61)), "a new window starts after the old one ends");
    for i in 0..5000 {
        l.allow(&format!("ip{i}"), t0);
    }
    l.allow("late", t0 + Duration::from_secs(120)); // the sweep runs on a big table
    assert!(l.tracked() <= 10, "quiet addresses must be forgotten, still {}", l.tracked());
}

#[test]
fn the_bucket_lets_a_burst_through_and_slows_what_comes_after() {
    let t0 = Instant::now();
    let b = ShareBucket::new(100, 1000, t0);
    assert_eq!(b.take(1000, t0), Duration::ZERO, "a burst up to the size of the bucket needs no wait");
    assert_eq!(b.take(100, t0), Duration::from_secs(1), "100 bytes over at 100 B/s wait one second");
    let t1 = t0 + Duration::from_secs(3); // the deficit (100) is paid and 200 more refilled
    assert_eq!(b.take(150, t1), Duration::ZERO);
    let t2 = t1 + Duration::from_secs(1000 * 3600); // a long silence never stores more than the burst
    assert_eq!(b.take(1000, t2), Duration::ZERO);
    assert!(b.take(1, t2) > Duration::ZERO, "the bucket must not hold more than its burst");
}

#[test]
fn the_bucket_has_defaults() {
    let b = ShareBucket::new(0, 0, Instant::now());
    assert_eq!((b.rate(), b.burst()), (DEFAULT_ROOM_BYTES_PER_SEC as f64, DEFAULT_ROOM_BURST as f64));
}

#[tokio::test]
async fn a_session_in_which_nothing_passes_is_ended() {
    let (mut plugin, mut bridge, _ctrl) = through(Config { idle_timeout: Duration::from_millis(200), ..Config::default() }).await;
    plugin.send(Message::Text("hello".into())).await.unwrap();
    assert_eq!(next(&mut bridge).await, Some(Message::Text("hello".into())));
    let start = Instant::now();
    assert!(matches!(next(&mut plugin).await, Some(Message::Close(_)) | None), "the idle session must end");
    let d = start.elapsed();
    assert!(d > Duration::from_millis(100) && d < Duration::from_secs(3), "ended after {d:?}, want about 200ms");
    assert!(matches!(next(&mut bridge).await, Some(Message::Close(_)) | None), "both ends must be closed");
}

#[tokio::test]
async fn a_frame_every_now_and_then_keeps_a_session_alive() {
    let (mut plugin, mut bridge, _ctrl) = through(Config { idle_timeout: Duration::from_millis(400), ..Config::default() }).await;
    for i in 0..5 {
        // 5 × 150 ms is longer than the idle limit, in total
        tokio::time::sleep(Duration::from_millis(150)).await;
        plugin.send(Message::Text("alive".into())).await.unwrap_or_else(|e| panic!("frame {i}: {e}"));
        assert!(matches!(next(&mut bridge).await, Some(Message::Text(_))), "frame {i} was not delivered");
    }
}

#[tokio::test]
async fn a_room_that_moves_more_than_its_share_is_slowed_not_cut() {
    let (mut plugin, mut bridge, _ctrl) =
        through(Config { room_bytes_per_sec: 100_000, room_burst: 10_000, ..Config::default() }).await;
    let frame = vec![0u8; 60_000];
    let start = Instant::now();
    let mut arrived = [Duration::ZERO; 2];
    for slot in arrived.iter_mut() {
        plugin.send(Message::Binary(frame.clone().into())).await.unwrap();
        match next(&mut bridge).await {
            Some(Message::Binary(b)) => assert_eq!(b.len(), frame.len()),
            other => panic!("expected the frame, got {other:?}"),
        }
        *slot = start.elapsed();
    }
    // 120 KB at 100 KB/s with a 10 KB burst: the second frame cannot arrive before ~0.5 s
    assert!(arrived[1] > Duration::from_millis(400), "the room was not slowed: {arrived:?}");
    assert!(arrived[1] < Duration::from_secs(5), "the room was cut or stalled: {arrived:?}");
}

#[tokio::test]
async fn the_log_names_no_client_and_no_whole_room() {
    let s = start(Config::default()).await;
    let room = room_id("secret");
    let mut ctrl = dial(s.addr, BRIDGE_PATH, &format!("room={room}")).await;
    let _plugin = dial(s.addr, CLIENT_PATH, &format!("room={room}")).await;
    let _ = next(&mut ctrl).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let lines = s.log.lock().unwrap().join("\n");
    assert!(lines.contains("bridge up"), "{lines}");
    assert!(!lines.contains(&room) && !lines.contains("127.0.0.1"), "the log must not name a room or an address:\n{lines}");
}

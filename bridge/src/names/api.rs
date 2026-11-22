//! The name service's HTTP API for bridges and relays (interfaces.md I28, I29). Admission first
//! (a per-address limit), then the body (4 KiB), then `Names` checks everything else.

use super::store::{ApiError, Names};
use crate::namesproto::{AcmeReq, MAX_BODY, RegisterReq};
use crate::relay::Limiter;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct Api {
    pub names: Arc<Names>,
    pub limiter: Arc<Limiter>,
    pub log: Arc<dyn Fn(&str) + Send + Sync>,
}

impl Api {
    pub fn new(names: Arc<Names>, log: Arc<dyn Fn(&str) + Send + Sync>) -> Api {
        // 60 a minute per address: a household registers once per start, an office a few times
        Api { names, limiter: Arc::new(Limiter::new(60, Duration::from_secs(60))), log }
    }
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn reply(r: Result<(), ApiError>, log: &(dyn Fn(&str) + Send + Sync)) -> Response {
    match r {
        Ok(()) => (StatusCode::OK, [("content-type", "application/json")], r#"{"ok":true}"#).into_response(),
        Err(e) => {
            if let ApiError::Internal(why) = &e {
                log(&format!("names: {why}"));
            }
            let body = match &e {
                ApiError::Quota(at) => serde_json::json!({"error": e.name(), "retryAfter": at}),
                _ => serde_json::json!({"error": e.name()}),
            }
            .to_string();
            (StatusCode::from_u16(e.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), [("content-type", "application/json")], body).into_response()
        }
    }
}

fn admit(api: &Api, peer: SocketAddr) -> Result<(), ApiError> {
    if api.limiter.allow(&peer.ip().to_string(), Instant::now()) { Ok(()) } else { Err(ApiError::TooMany("rate_limited")) }
}

async fn register(State(api): State<Api>, ConnectInfo(peer): ConnectInfo<SocketAddr>, body: Bytes) -> Response {
    let r = admit(&api, peer).and_then(|()| serde_json::from_slice::<RegisterReq>(&body).map_err(|_| ApiError::BadRequest("bad_json"))).and_then(|req| api.names.register(&req, now()));
    reply(r, api.log.as_ref())
}

async fn acme(State(api): State<Api>, ConnectInfo(peer): ConnectInfo<SocketAddr>, body: Bytes) -> Response {
    let r = admit(&api, peer).and_then(|()| serde_json::from_slice::<AcmeReq>(&body).map_err(|_| ApiError::BadRequest("bad_json"))).and_then(|req| api.names.acme(&req, now()));
    reply(r, api.log.as_ref())
}

async fn health(State(api): State<Api>) -> Response {
    (StatusCode::OK, [("content-type", "application/json")], format!(r#"{{"ok":true,"records":{}}}"#, api.names.len())).into_response()
}

pub fn router(api: Api) -> Router {
    Router::new().route("/v1/register", post(register)).route("/v1/acme", post(acme)).route("/health", get(health)).layer(DefaultBodyLimit::max(MAX_BODY)).with_state(api)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::names::store::Limits;
    use crate::namesproto::Identity;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn app() -> (Router, Arc<Names>, Identity, tempfile::TempDir) {
        let d = tempfile::tempdir().unwrap();
        let names = Arc::new(Names::new(None, Limits::default()));
        let me = Identity::load_or_create(d.path()).unwrap();
        (router(Api::new(names.clone(), Arc::new(|_| {}))), names, me, d)
    }

    async fn post_json(app: &Router, path: &str, body: Vec<u8>, from: &str) -> (u16, String) {
        let mut req = Request::post(path).body(Body::from(body)).unwrap();
        req.extensions_mut().insert(ConnectInfo::<SocketAddr>(from.parse().unwrap()));
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status().as_u16();
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn register_then_publish_a_txt_value() {
        let (app, names, me, _d) = app();
        let now = now();
        let (s, b) = post_json(&app, "/v1/register", serde_json::to_vec(&me.register("192.168.1.9", now)).unwrap(), "203.0.113.1:1").await;
        assert_eq!((s, b.as_str()), (200, r#"{"ok":true}"#));
        assert!(names.exists(me.id()));
        let v = "q".repeat(43);
        let (s, _) = post_json(&app, "/v1/acme", serde_json::to_vec(&me.acme("set", &v, now)).unwrap(), "203.0.113.1:1").await;
        assert_eq!(s, 200);
        assert_eq!(names.txt(me.id(), now), vec![v]);
    }

    #[tokio::test]
    async fn bad_requests_get_a_status_and_a_short_name_and_write_nothing() {
        let (app, names, me, _d) = app();
        let now = now();
        let (s, b) = post_json(&app, "/v1/register", b"not json".to_vec(), "203.0.113.1:1").await;
        assert_eq!((s, b.as_str()), (400, r#"{"error":"bad_json"}"#));
        let (s, _) = post_json(&app, "/v1/register", br#"{"id":"x","unknown":1}"#.to_vec(), "203.0.113.1:1").await;
        assert_eq!(s, 400, "unknown fields are refused");
        let (s, b) = post_json(&app, "/v1/register", serde_json::to_vec(&me.register("8.8.8.8", now)).unwrap(), "203.0.113.1:1").await;
        assert_eq!((s, b.as_str()), (403, r#"{"error":"ip_not_private"}"#));
        let (s, b) = post_json(&app, "/v1/acme", serde_json::to_vec(&me.acme("set", &"q".repeat(43), now)).unwrap(), "203.0.113.1:1").await;
        assert_eq!((s, b.as_str()), (404, r#"{"error":"unknown_id"}"#), "the bridge reads this as 'register again'");
        assert_eq!(names.len(), 0);
    }

    #[tokio::test]
    async fn a_large_body_is_refused_before_it_is_read() {
        let (app, _n, _me, _d) = app();
        let (s, _) = post_json(&app, "/v1/register", vec![b' '; 5000], "203.0.113.1:1").await;
        assert_eq!(s, 413);
    }

    #[tokio::test]
    async fn one_address_is_limited_and_another_is_not() {
        let (app, _n, _me, _d) = app();
        for _ in 0..60 {
            let (s, _) = post_json(&app, "/v1/register", b"{}".to_vec(), "203.0.113.5:1").await;
            assert_eq!(s, 400);
        }
        let (s, b) = post_json(&app, "/v1/register", b"{}".to_vec(), "203.0.113.5:1").await;
        assert_eq!((s, b.as_str()), (429, r#"{"error":"rate_limited"}"#));
        let (s, _) = post_json(&app, "/v1/register", b"{}".to_vec(), "203.0.113.6:1").await;
        assert_eq!(s, 400);
    }
}

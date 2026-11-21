//! The HTTP front: the liveness probe, the WebSocket at /v1, and the local-only doors.

use super::Deps;
use super::admin::AdminDeps;
use super::session::{MAX_FRAME_BYTES, Session};
use super::transport::Transport;
use crate::protocol;
use axum::Router;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{ConnectInfo, FromRequest, Request};
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use std::net::SocketAddr;
use std::sync::Arc;

/// Anything arriving through a proxy or tunnel (e.g. Cloudflare) has crossed the internet: only the
/// end-to-end encrypted handshake is accepted there.
fn proxied(h: &HeaderMap) -> bool {
    crate::localguard::via_proxy(h)
}

fn query_param(req: &Request, name: &str) -> String {
    req.uri().query().unwrap_or("").split('&').filter_map(|kv| kv.split_once('=')).find(|(k, _)| *k == name).map(|(_, v)| v.to_string()).unwrap_or_default()
}

/// Serves GET /health (unauthenticated liveness probe) and the WebSocket at /v1 (and, with
/// `admin`, the /admin/ routes). `extra` adds routes such as the local pages.
pub fn router(deps: Arc<Deps>, admin: Option<AdminDeps>, extra: Option<Router>) -> Router {
    let d = deps.clone();
    let mut app = Router::new().route("/health", get(|| async { ([(header::CONTENT_TYPE, "application/json")], format!("{{\"ok\":true,\"protocol\":{}}}\n", protocol::VERSION)) })).route(
        protocol::WS_PATH,
        get(move |ci: ConnectInfo<SocketAddr>, req: Request| {
            let deps = d.clone();
            async move { upgrade(deps, ci.0, req).await }
        }),
    );
    if let Some(a) = admin {
        app = app.merge(a.router());
    }
    if let Some(e) = extra {
        app = app.merge(e);
    }
    app
}

async fn upgrade(deps: Arc<Deps>, peer: SocketAddr, req: Request) -> Response {
    // The plugin is not a browser, so there is no Origin to check; the token authenticates.
    let encrypted_only = proxied(req.headers());
    let device = query_param(&req, "d");
    let up = match WebSocketUpgrade::from_request(req, &()).await {
        Ok(u) => u,
        Err(e) => return e.into_response(),
    };
    up.max_message_size(MAX_FRAME_BYTES).max_frame_size(MAX_FRAME_BYTES).on_upgrade(move |ws| async move {
        Session::serve(deps, Transport::from_axum(ws), &peer.to_string(), encrypted_only, &device).await;
    })
}

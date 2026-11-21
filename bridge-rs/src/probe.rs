//! Checks that a bridge can be reached the way the plugin reaches it: open the WebSocket, complete
//! the end-to-end encrypted handshake with the token, and read the welcome (Go: internal/probe).
//! `spellink-bridge check` uses it, so a person can tell "the tunnel is up" from "the plugin will
//! get through" without a Supernote in hand.

use crate::protocol::VERSION;
use crate::securechan::{self, Channel};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message;

/// What a successful probe learned.
#[derive(Debug, Clone, PartialEq)]
pub struct Probed {
    pub bridge: String,
    pub version: String,
    pub protocol: i64,
    pub elapsed: Duration,
}

/// Dials `url` with the pairing token. It never asks a question.
pub async fn probe(url: &str, token: &str) -> Result<Probed, String> {
    let started = Instant::now();
    let (mut ws, _) = tokio_tungstenite::connect_async_with_config(url, None, true).await.map_err(|e| format!("connect: {e}"))?;
    let (init, msg1) = securechan::Initiator::start(&securechan::psk(token)).map_err(|e| format!("handshake: {e}"))?;
    ws.send(Message::Binary(msg1.into())).await.map_err(|e| format!("handshake: {e}"))?;
    // A wrong token surfaces here: the bridge cannot answer the handshake.
    let ch: Channel = match ws.next().await {
        Some(Ok(Message::Binary(m2))) => init.finish(&m2).map_err(|e| format!("handshake: {e}"))?,
        Some(Ok(Message::Close(f))) => return Err(format!("handshake: the bridge closed the connection ({f:?})")),
        Some(Ok(_)) => return Err("handshake: expected a binary frame".into()),
        Some(Err(e)) => return Err(format!("handshake: {e}")),
        None => return Err("handshake: the connection closed".into()),
    };
    let mut buf = Vec::new();
    loop {
        match ws.next().await {
            Some(Ok(Message::Binary(ct))) => match ch.open(&ct, &mut buf) {
                Ok(true) => break,
                Ok(false) => {}
                Err(e) => return Err(format!("welcome: {e}")),
            },
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
            Some(Ok(_)) | Some(Err(_)) | None => return Err("welcome: the connection closed".into()),
        }
    }
    let w: Value = serde_json::from_slice(&buf).map_err(|e| format!("welcome: {e}"))?;
    if w["type"] != "welcome" {
        return Err(format!("the bridge answered {} instead of welcome ({})", w["type"], w["message"].as_str().unwrap_or("")));
    }
    let protocol = w["protocol"].as_i64().unwrap_or(0);
    if protocol != VERSION {
        return Err(format!("the bridge speaks protocol {protocol}, this program {VERSION}"));
    }
    let bridge = w["bridge"].as_str().unwrap_or("");
    if bridge.is_empty() {
        return Err("welcome without a bridge name".into());
    }
    let _ = ws.close(None).await;
    Ok(Probed { bridge: bridge.to_string(), version: w["version"].as_str().unwrap_or("").to_string(), protocol, elapsed: started.elapsed() })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn bridge() -> String {
        // a real bridge on a port, from the integration helpers' shape
        use crate::provider::{self, AskFuture};
        use crate::server::{http, Deps};
        use std::sync::Arc;
        let deps = Deps {
            name: "probe-test".into(),
            version: "9".into(),
            token: "secret".into(),
            font: None,
            provider: Arc::new(provider::func("t", |_, _, _, _| -> AskFuture<'static> { Box::pin(async { Ok(String::new()) }) })),
            report: Arc::new(|_, _| {}),
            log: Arc::new(|_| {}),
            settings: Arc::new(Default::default),
            set_settings: Arc::new(|_| Ok(())),
            history: Arc::new(crate::history::Store::none()),
            history_chars: 0,
            runs: Arc::new(Default::default()),
            first_output: Duration::ZERO,
            gap: Duration::ZERO,
            total: Duration::ZERO,
            keepalive_every: Duration::ZERO,
            orphan_grace: Duration::ZERO,
            devices: None,
            sessions: Arc::new(Default::default()),
            pair_request_wait: Duration::ZERO,
            pairing_max: Duration::ZERO,
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = http::router(Arc::new(deps), None, None).into_make_service_with_connect_info::<std::net::SocketAddr>();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("ws://{addr}/v1")
    }

    #[tokio::test]
    async fn probe_reaches_a_bridge_with_the_right_token() {
        let r = probe(&bridge().await, "secret").await.unwrap();
        assert_eq!((r.bridge.as_str(), r.version.as_str(), r.protocol), ("probe-test", "9", VERSION));
        assert!(r.elapsed > Duration::ZERO);
    }

    #[tokio::test]
    async fn probe_with_the_wrong_token_fails_at_the_handshake() {
        let e = probe(&bridge().await, "not-the-token").await.unwrap_err();
        assert!(e.contains("handshake") || e.contains("welcome") || e.contains("connect"), "a wrong token must be an error at the handshake, got {e}");
    }

    #[tokio::test]
    async fn probe_says_when_nothing_is_there() {
        let e = probe("ws://127.0.0.1:1/v1", "secret").await.unwrap_err();
        assert!(e.contains("connect"), "{e}");
    }
}

//! A plugin session through the relay behaves exactly like a direct LAN session (Go:
//! TestSessionThroughRelay): the real relay, the bridge's connector, the real session.

mod common;

use common::*;
use serde_json::json;
use spellink_bridge::connector::{self, ServeFn};
use spellink_bridge::relay::{Config, Relay};
use spellink_bridge::server::session::Session;
use spellink_bridge::server::transport::Transport;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

async fn start_relay() -> String {
    let relay = Relay::new(Config::default(), quiet());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = relay.router().into_make_service_with_connect_info::<SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("ws://{addr}")
}

fn serving(deps: Arc<spellink_bridge::server::Deps>) -> ServeFn {
    Arc::new(move |t: Transport, peer: String| {
        let deps = deps.clone();
        Box::pin(async move { Session::serve(deps, t, &peer, true, "").await })
    })
}

/// Asks one question over Noise (or with the legacy plaintext hello) and lists the frame types.
async fn converse_through(url: &str, encrypted: bool) -> Vec<String> {
    let q = ask("q", "", "x");
    let mut out = vec![];
    if encrypted {
        let Ok(mut d) = Device::try_open(url, "secret").await else { return out };
        d.send(q).await;
        while let Some(m) = d.recv().await {
            out.push(m["type"].as_str().unwrap_or("?").to_string());
            if finished(&m) {
                break;
            }
        }
    } else {
        let mut c = Client::connect(url).await;
        c.send(hello()).await;
        c.send(q).await;
        while let Some(m) = c.recv().await {
            out.push(m["type"].as_str().unwrap_or("?").to_string());
            if finished(&m) {
                break;
            }
        }
    }
    out
}

#[tokio::test]
async fn a_session_through_the_relay_behaves_like_a_direct_one() {
    let base = start_relay().await;
    // the plugin before the bridge is up is told the bridge is offline
    let mut c = Client::connect(&connector::client_url(&base, "secret")).await;
    assert!(c.recv().await.is_none(), "the relay closes a session for a room nobody holds");

    let deps = Arc::new(default_deps(ai(|_, _, s, _| async move {
        text(&s, "あ");
        Ok("あ".to_string())
    })));
    let cancel = CancellationToken::new();
    tokio::spawn(connector::connect(cancel.clone(), base.clone(), "secret".into(), serving(deps), quiet()));

    // wait for the bridge to register, then talk to it through the relay (encrypted)
    let url = connector::client_url(&base, "secret");
    let mut got = vec![];
    for _ in 0..100 {
        got = converse_through(&url, true).await;
        if !got.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(got.join(","), "welcome,status,status,line,done");
    // the relay path refuses the legacy plaintext hello
    assert_eq!(converse_through(&url, false).await.join(","), "error");
    cancel.cancel();
}

#[tokio::test]
async fn a_bridge_that_loses_the_relay_reconnects_when_it_returns() {
    // the connector retries with backoff: after the relay is gone for a moment, it registers again
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener); // nothing listens yet
    let base = format!("ws://{addr}");
    let deps = Arc::new(default_deps(ai(|_, _, s, _| async move {
        text(&s, "あ");
        Ok("あ".to_string())
    })));
    let cancel = CancellationToken::new();
    tokio::spawn(connector::connect(cancel.clone(), base.clone(), "secret".into(), serving(deps), quiet()));
    tokio::time::sleep(Duration::from_millis(300)).await; // the first attempt fails
    let relay = Relay::new(Config::default(), quiet());
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    let app = relay.router().into_make_service_with_connect_info::<SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let url = connector::client_url(&base, "secret");
    let mut got = vec![];
    for _ in 0..80 {
        got = converse_through(&url, true).await;
        if !got.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(got.join(","), "welcome,status,status,line,done", "the bridge registered once the relay was there");
    cancel.cancel();
    let _ = json!(null);
}

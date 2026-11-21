//! Helpers shared by the integration tests: a bridge on a real port, a plugin that talks to it.
#![allow(dead_code)]

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use spellink_bridge::glyphs::{Font, Glyph};
use spellink_bridge::history;
use spellink_bridge::protocol::Settings;
use spellink_bridge::provider::{self, AskError, AskFuture, Provider, Sink};
use spellink_bridge::server::run::Runs;
use spellink_bridge::server::session::Sessions;
use spellink_bridge::server::{http, Deps};
use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tokio_util::sync::CancellationToken;

pub type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub fn layout() -> Value {
    json!({"x": 10, "y": 10, "width": 400, "pageHeight": 1000, "bottomMargin": 20, "charSize": 40})
}

pub fn hello() -> Value {
    json!({"type": "hello", "token": "secret", "client": "t"})
}

pub fn ask(id: &str, note: &str, q: &str) -> Value {
    json!({"type": "ask", "id": id, "note": note, "question": q, "lang": "ja", "layout": layout()})
}

pub fn quiet() -> Arc<dyn Fn(&str) + Send + Sync> {
    Arc::new(|_| {})
}

pub fn test_font() -> Arc<Font> {
    let mut glyphs = HashMap::new();
    glyphs.insert('あ', Glyph { advance: 100, strokes: vec![vec![50, 0, 50, 100]] });
    Arc::new(Font { em: 100, sources: vec![], glyphs })
}

/// A test AI from an async function of (question, system prompt, sink, cancel). `text` also
/// reports a sign of life, as a real provider's text does.
pub fn ai<F, Fut>(f: F) -> Arc<dyn Provider>
where
    F: Fn(String, String, Sink, CancellationToken) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<String, AskError>> + Send + 'static,
{
    Arc::new(provider::func("test AI", move |q, sys, s, c| -> AskFuture<'static> { Box::pin(f(q, sys, s, c)) }))
}

/// Gives the text and, as a provider that only streams text does, a sign of life.
pub fn text(s: &Sink, t: &str) {
    (s.activity)(true);
    (s.text)(t);
}

pub fn default_deps(p: Arc<dyn Provider>) -> Deps {
    Deps {
        name: "test".into(),
        version: "0".into(),
        token: "secret".into(),
        font: Some(test_font()),
        provider: p,
        report: Arc::new(|_, _| {}),
        log: quiet(),
        settings: Arc::new(Settings::default),
        set_settings: Arc::new(|_| Ok(())),
        history: Arc::new(history::Store::none()),
        history_chars: 0,
        runs: Arc::new(Runs::default()),
        first_output: Duration::ZERO,
        gap: Duration::ZERO,
        total: Duration::ZERO,
        keepalive_every: Duration::ZERO,
        orphan_grace: Duration::ZERO,
        devices: None,
        sessions: Arc::new(Sessions::default()),
        pair_request_wait: Duration::ZERO,
        pairing_max: Duration::ZERO,
    }
}

/// Serves the deps on a port and returns the WebSocket URL.
pub async fn serve(deps: Deps) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = http::router(Arc::new(deps), None, None).into_make_service_with_connect_info::<SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("ws://{addr}/v1")
}

pub async fn start(p: Arc<dyn Provider>) -> String {
    serve(default_deps(p)).await
}

pub async fn start_with(p: Arc<dyn Provider>, tweak: impl FnOnce(&mut Deps)) -> String {
    let mut d = default_deps(p);
    tweak(&mut d);
    serve(d).await
}

pub struct Client {
    pub ws: Ws,
}

impl Client {
    pub async fn connect(url: &str) -> Client {
        Client { ws: tokio_tungstenite::connect_async(url).await.expect("connect").0 }
    }

    pub async fn connect_with_header(url: &str, name: &'static str, value: &str) -> Client {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let mut req = url.into_client_request().unwrap();
        req.headers_mut().insert(name, value.parse().unwrap());
        Client { ws: tokio_tungstenite::connect_async(req).await.expect("connect").0 }
    }

    pub async fn send(&mut self, v: Value) {
        self.ws.send(Message::Text(v.to_string().into())).await.expect("send");
    }

    /// The next frame, or None when the socket closed (or nothing came within 5 s).
    pub async fn recv(&mut self) -> Option<Value> {
        self.recv_within(Duration::from_secs(5)).await
    }

    pub async fn recv_within(&mut self, d: Duration) -> Option<Value> {
        loop {
            match tokio::time::timeout(d, self.ws.next()).await {
                Ok(Some(Ok(Message::Text(t)))) => return serde_json::from_str(&t).ok(),
                Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => continue,
                _ => return None,
            }
        }
    }

    /// Reads frames until one satisfies `until` (and returns what was read, it included).
    pub async fn until(&mut self, until: impl Fn(&Value) -> bool) -> Vec<Value> {
        let mut got = vec![];
        while let Some(m) = self.recv().await {
            let stop = until(&m);
            got.push(m);
            if stop {
                break;
            }
        }
        got
    }

    /// Every frame that arrives until nothing comes for `quiet`.
    pub async fn until_quiet(&mut self, quiet: Duration) -> Vec<Value> {
        let mut out = vec![];
        while let Some(m) = self.recv_within(quiet).await {
            out.push(m);
        }
        out
    }

    /// Says hello and reads the welcome.
    pub async fn dial(url: &str) -> Client {
        let mut c = Client::connect(url).await;
        c.send(hello()).await;
        let w = c.recv().await.expect("welcome");
        assert_eq!(w["type"], "welcome", "{w}");
        c
    }

    /// Reads frames until the answer ends and returns the terminating one.
    pub async fn last_frame(&mut self) -> Value {
        loop {
            let m = self.recv().await.expect("the answer ended");
            if m["type"] == "done" || m["type"] == "error" {
                return m;
            }
        }
    }
}

pub fn finished(m: &Value) -> bool {
    m["type"] == "done" || m["type"] == "error"
}

pub fn types(ms: &[Value]) -> String {
    ms.iter().map(|m| m["type"].as_str().unwrap_or("?").to_string()).collect::<Vec<_>>().join(",")
}

/// Sends frames in order on a new connection and collects replies until `until` matches or the
/// socket closes.
pub async fn converse(url: &str, frames: Vec<Value>, until: impl Fn(&Value) -> bool) -> Vec<Value> {
    let mut c = Client::connect(url).await;
    for f in frames {
        c.send(f).await;
    }
    c.until(until).await
}

pub fn new_log() -> (Arc<dyn Fn(&str) + Send + Sync>, Arc<Mutex<Vec<String>>>) {
    let lines = Arc::new(Mutex::new(Vec::new()));
    let l2 = lines.clone();
    (Arc::new(move |s| l2.lock().unwrap().push(s.to_string())), lines)
}

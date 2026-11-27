//! Helpers shared by the integration tests: a bridge on a real port, a plugin that talks to it.
#![allow(dead_code)]

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use spellink_bridge::glyphs::{Font, Glyph};
use spellink_bridge::history;
use spellink_bridge::protocol::Settings;
use spellink_bridge::provider::{self, AskError, AskFuture, Provider, Sink};
use spellink_bridge::server::run::Runs;
use spellink_bridge::server::session::Sessions;
use spellink_bridge::server::{Deps, http};
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

// ---- devices: the plugin's end of a device session, and the command line's door ----

use spellink_bridge::devices::{self, Hooks, Registry, Store, registry::PackageData};
use spellink_bridge::securechan::{self, Channel};
use spellink_bridge::server::admin::AdminDeps;

pub const ADMIN_TOKEN: &str = "admin-token-for-tests";

/// The real file store, made unwritable on demand.
pub struct FailingStore {
    pub inner: devices::FileStore,
    pub fail: Arc<Mutex<Option<String>>>,
}

impl Store for FailingStore {
    fn load(&self) -> Result<Option<Vec<u8>>, String> {
        self.inner.load()
    }
    fn save(&self, b: &[u8]) -> Result<(), String> {
        if let Some(e) = self.fail.lock().unwrap().clone() {
            return Err(e);
        }
        self.inner.save(b)
    }
}

pub struct PairEnv {
    pub reg: Registry,
    pub sessions: Arc<Sessions>,
    pub base: String, // ws://…/v1
    pub http: String, // host:port
    pub fail: Arc<Mutex<Option<String>>>,
    _dir: tempfile::TempDir,
}

pub fn new_ledger(dir: &std::path::Path, sessions: &Arc<Sessions>) -> (Registry, Arc<Mutex<Option<String>>>) {
    let fail = Arc::new(Mutex::new(None));
    let store = FailingStore { inner: devices::FileStore::new(dir.join("devices.json")), fail: fail.clone() };
    let s2 = sessions.clone();
    let hooks = Hooks { endpoints: Some(Arc::new(|id, _| vec![format!("wss://bridge.example/v1?d={id}")])), changed: None, close_all: Some(Arc::new(move |id| s2.close_device(id))) };
    let (reg, _) = Registry::new(devices::Config::default(), Box::new(store), hooks, devices::os_rand()).expect("ledger");
    (reg, fail)
}

pub fn default_admin(reg: &Registry) -> AdminDeps {
    AdminDeps { devices: reg.clone(), token: ADMIN_TOKEN.into(), package: Arc::new(|d: PackageData| Ok(format!("package for {} gen {} B={}", d.name, d.generation, d.b).into_bytes())), status: None, ai: None }
}

impl PairEnv {
    pub async fn new() -> PairEnv {
        PairEnv::with(|_, _| {}).await
    }

    pub async fn with(tweak: impl FnOnce(&mut Deps, &mut Option<AdminDeps>)) -> PairEnv {
        let dir = tempfile::tempdir().unwrap();
        let sessions = Arc::new(Sessions::default());
        let (reg, fail) = new_ledger(dir.path(), &sessions);
        let mut d = default_deps(ai(|_, _, _, _| async { Ok(String::new()) }));
        d.devices = Some(reg.clone());
        d.sessions = sessions.clone();
        let mut admin = Some(default_admin(&reg));
        tweak(&mut d, &mut admin);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = http::router(Arc::new(d), admin, None).into_make_service_with_connect_info::<SocketAddr>();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        PairEnv { reg, sessions, base: format!("ws://{addr}/v1"), http: addr.to_string(), fail, _dir: dir }
    }

    pub fn add_device(&self, name: &str) -> (String, String) {
        let id = self.reg.add(name).expect("add");
        let d = self.reg.repack(&id).expect("repack");
        (id, d.b)
    }

    pub async fn wait_waiting(&self, id: &str, n: usize) {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if self.reg.list().iter().any(|i| i.id == id && i.waiting == n) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("device {id} never had {n} waiting request(s)");
    }

    /// Opens a device session with `secret` (the handshake must succeed).
    pub async fn dial(&self, id: &str, secret: &str) -> Device {
        Device::try_open(&format!("{}?d={id}", self.base), secret).await.expect("handshake")
    }

    /// A request to the command line's door: (status, body).
    pub async fn admin(&self, method: &str, path: &str, body: Option<Value>) -> (u16, String) {
        http_request(&self.http, method, path, &[("Authorization", &format!("Bearer {ADMIN_TOKEN}"))], body.map(|b| b.to_string())).await
    }

    /// Takes a device through the whole ceremony and returns its id and K.
    pub async fn pair_fully(&self, name: &str, code: &str) -> (String, String) {
        let (id, b) = self.add_device(name);
        let mut d = self.dial(&id, &b).await;
        d.send(json!({"type": "pairRequest", "id": id, "gen": 1, "code": code})).await;
        self.wait_waiting(&id, 1).await;
        let (status, _) = self.admin("POST", "/admin/pair", Some(json!({"code": code}))).await;
        assert_eq!(status, 200, "confirm");
        let g = d.recv().await.expect("a grant");
        assert_eq!(g["type"], "pairGrant", "{g}");
        d.send(json!({"type": "pairAck", "id": id, "gen": 1})).await;
        assert!(d.recv().await.is_none(), "the pairing channel must end after the ack");
        (id, g["key"].as_str().unwrap().to_string())
    }
}

/// A plugin's end of an encrypted session.
pub struct Device {
    ws: Ws,
    ch: Channel,
}

impl Device {
    /// Dials and shakes hands; an error says the bridge refused.
    pub async fn try_open(url: &str, secret: &str) -> Result<Device, String> {
        let mut ws = tokio_tungstenite::connect_async(url).await.map_err(|e| e.to_string())?.0;
        let (init, m1) = securechan::Initiator::start(&securechan::psk(secret)).map_err(|e| e.to_string())?;
        ws.send(Message::Binary(m1.into())).await.map_err(|e| e.to_string())?;
        let reply = tokio::time::timeout(Duration::from_secs(5), ws.next()).await.map_err(|_| "no answer".to_string())?;
        match reply {
            Some(Ok(Message::Binary(m2))) => Ok(Device { ws, ch: init.finish(&m2).map_err(|e| e.to_string())? }),
            other => Err(format!("the bridge refused: {other:?}")),
        }
    }

    pub async fn send(&mut self, v: Value) {
        let mut chunks = vec![];
        self.ch
            .seal(v.to_string().as_bytes(), |c| {
                chunks.push(c);
                Ok(())
            })
            .unwrap();
        for c in chunks {
            self.ws.send(Message::Binary(c.into())).await.expect("send");
        }
    }

    /// The next frame, or None if the session ended.
    pub async fn recv(&mut self) -> Option<Value> {
        let mut buf = Vec::new();
        loop {
            match tokio::time::timeout(Duration::from_secs(5), self.ws.next()).await {
                Ok(Some(Ok(Message::Binary(ct)))) => match self.ch.open(&ct, &mut buf) {
                    Ok(true) => return serde_json::from_slice(&buf).ok(),
                    Ok(false) => {}
                    Err(_) => return None,
                },
                Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => {}
                _ => return None,
            }
        }
    }
}

/// A plain HTTP/1.1 request on a fresh connection; (status, body).
pub async fn http_request(addr: &str, method: &str, path: &str, headers: &[(&str, &str)], body: Option<String>) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = TcpStream::connect(addr).await.unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\n");
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
        req += &format!("Host: {addr}\r\n");
    }
    for (k, v) in headers {
        req += &format!("{k}: {v}\r\n");
    }
    req += "Connection: close\r\n";
    if let Some(b) = &body {
        req += &format!("Content-Type: application/json\r\nContent-Length: {}\r\n", b.len());
    }
    req += "\r\n";
    s.write_all(req.as_bytes()).await.unwrap();
    if let Some(b) = body {
        s.write_all(b.as_bytes()).await.unwrap();
    }
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, rest) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status: u16 = head.split(' ').nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let body = if head.to_lowercase().contains("transfer-encoding: chunked") { dechunk(rest) } else { rest.to_string() };
    (status, body)
}

fn dechunk(mut s: &str) -> String {
    let mut out = String::new();
    while let Some((size, rest)) = s.split_once("\r\n") {
        let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
        if n == 0 {
            break;
        }
        out.push_str(&rest[..n]);
        s = &rest[n + 2..];
    }
    out
}

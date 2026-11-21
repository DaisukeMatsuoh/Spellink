//! A relay that lets a plugin outside the LAN reach a bridge behind NAT (Go: internal/relay).
//! Both sides dial out to the relay, which pairs them by room and copies WebSocket frames between
//! them without interpreting them (end-to-end encryption is the bridge's and plugin's job,
//! ADR 0007/0010).
//!
//! ```text
//! bridge ──/v1/relay/bridge?room=R──▶ relay   (control: one per bridge, long-lived)
//! plugin ──/v1/relay/client?room=R──▶ relay ──{"type":"session","id":S}──▶ bridge
//! bridge ──/v1/relay/accept?room=R&session=S──▶ relay  ⇄ frames ⇄ plugin
//! ```

use axum::Router;
use axum::body::Body;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, FromRequest, Request};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};

pub const BRIDGE_PATH: &str = "/v1/relay/bridge";
pub const CLIENT_PATH: &str = "/v1/relay/client";
pub const ACCEPT_PATH: &str = "/v1/relay/accept";

/// Close codes sent to the plugin.
pub const STATUS_BRIDGE_OFFLINE: u16 = 4404;
pub const STATUS_ROOM_BUSY: u16 = 4429;
/// Sent to a bridge when the relay has no room left for a new one.
pub const STATUS_RELAY_FULL: u16 = 4503;
const STATUS_GOING_AWAY: u16 = 1001;
const STATUS_NORMAL: u16 = 1000;
const STATUS_POLICY_VIOLATION: u16 = 1008;

const MAX_FRAME_BYTES: usize = 1 << 20;
const MAX_SESSIONS_PER_ROOM: usize = 4;
/// Bounds the memory anyone can make the relay use: a room is created by any connection naming a
/// new id, and ids are free to invent.
const MAX_ROOMS: usize = 10_000;
const ACCEPT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(15 * 60);
pub const DEFAULT_ROOM_BYTES_PER_SEC: i64 = 4 << 20;
pub const DEFAULT_ROOM_BURST: i64 = 16 << 20;

/// Derives the relay room from the bridge token. The relay only ever sees this hash, never the
/// token itself.
pub fn room_id(token: &str) -> String {
    let h = Sha256::digest(format!("spellink-room-v1:{token}").as_bytes());
    hex::encode(&h[..16])
}

fn valid_room(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

// ---- limits ----

/// Allows each source address a number of attempts per window: a fixed window per address, enough
/// to keep one address from using a relay up. Addresses that have gone quiet are forgotten, so the
/// table cannot grow without bound either.
pub struct Limiter {
    max: usize,
    window: Duration,
    hits: Mutex<HashMap<String, (Instant, usize)>>,
}

impl Limiter {
    pub fn new(max: usize, window: Duration) -> Self {
        Limiter { max, window, hits: Mutex::new(HashMap::new()) }
    }

    /// Records an attempt from `key` at `now` and reports whether it is within the limit.
    pub fn allow(&self, key: &str, now: Instant) -> bool {
        let mut hits = self.hits.lock().unwrap();
        if hits.len() > 4096 {
            let window = self.window;
            hits.retain(|_, (start, _)| now.saturating_duration_since(*start) <= window);
        }
        let e = hits.entry(key.to_string()).or_insert((now, 0));
        if now.saturating_duration_since(e.0) >= self.window {
            *e = (now, 0);
        }
        e.1 += 1;
        e.1 <= self.max
    }

    pub fn tracked(&self) -> usize {
        self.hits.lock().unwrap().len()
    }
}

/// A token bucket: `rate` bytes per second refill it up to `burst`. One room's share of bandwidth.
pub struct ShareBucket {
    rate: f64,
    burst: f64,
    state: Mutex<(f64, Instant)>,
}

impl ShareBucket {
    pub fn new(rate: i64, burst: i64, now: Instant) -> Self {
        let rate = if rate <= 0 { DEFAULT_ROOM_BYTES_PER_SEC } else { rate } as f64;
        let burst = if burst <= 0 { DEFAULT_ROOM_BURST } else { burst } as f64;
        ShareBucket { rate, burst, state: Mutex::new((burst, now)) }
    }

    pub fn rate(&self) -> f64 {
        self.rate
    }
    pub fn burst(&self) -> f64 {
        self.burst
    }

    /// Spends `n` bytes and says how long to wait before sending more (zero: no need).
    pub fn take(&self, n: u64, now: Instant) -> Duration {
        let mut st = self.state.lock().unwrap();
        let elapsed = now.saturating_duration_since(st.1).as_secs_f64();
        st.0 = self.burst.min(st.0 + elapsed * self.rate);
        st.1 = now;
        st.0 -= n as f64;
        if st.0 >= 0.0 { Duration::ZERO } else { Duration::from_secs_f64(-st.0 / self.rate) }
    }
}

/// Names who is connecting. Behind a proxy or tunnel (Cloudflare) the connection comes from the
/// proxy, so the real address is in a header — but only if the operator said the proxy can be
/// trusted: a header sent by anyone else would let a client pick its own identity.
fn source_of(headers: &HeaderMap, peer: SocketAddr, trust_proxy: bool) -> String {
    if trust_proxy {
        if let Some(ip) = headers.get("cf-connecting-ip").and_then(|v| v.to_str().ok()).map(str::trim) {
            if !ip.is_empty() {
                return ip.to_string();
            }
        }
        if let Some(x) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
            if !x.is_empty() {
                return x.split(',').next().unwrap_or("").trim().to_string();
            }
        }
    }
    peer.ip().to_string()
}

// ---- the server ----

enum Ctrl {
    /// Tell the bridge a plugin wants a session; the answer says whether the frame was written.
    Session { frame: String, written: oneshot::Sender<bool> },
    /// Another control connection took the room.
    Replaced,
}

struct CtrlHandle {
    epoch: u64,
    tx: mpsc::Sender<Ctrl>,
}

struct Room {
    bucket: Arc<ShareBucket>,
    ctrl: Option<CtrlHandle>,
    pending: HashMap<String, oneshot::Sender<WebSocket>>,
    sessions: usize,
}

/// What an operator can set. Public so tests can use small values.
#[derive(Clone)]
pub struct Config {
    pub max_rooms: usize,
    /// Take the client's address from Cf-Connecting-Ip / X-Forwarded-For. Set it only when the
    /// relay is reachable solely through such a proxy or tunnel.
    pub trust_proxy: bool,
    /// Ends a session in which no frame has passed for this long. An answer being written sends
    /// a frame every few seconds, so a live session is never idle.
    pub idle_timeout: Duration,
    /// One room's share of bandwidth, counting both directions (defaults 4 MiB/s and 16 MiB: an
    /// answer is hundreds of KB). Beyond it the room is slowed, not cut.
    pub room_bytes_per_sec: i64,
    pub room_burst: i64,
    /// Attempts per address per minute, per kind of connection: a plugin connects once per
    /// question (and when reconnecting); a bridge keeps one control connection and opens one more
    /// per plugin session.
    pub clients_per_min: usize,
    pub bridges_per_min: usize,
    pub accepts_per_min: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            max_rooms: MAX_ROOMS,
            trust_proxy: false,
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
            room_bytes_per_sec: DEFAULT_ROOM_BYTES_PER_SEC,
            room_burst: DEFAULT_ROOM_BURST,
            clients_per_min: 60,
            bridges_per_min: 12,
            accepts_per_min: 120,
        }
    }
}

type LogFn = Arc<dyn Fn(&str) + Send + Sync>;

struct Inner {
    cfg: Config,
    log: LogFn,
    clients: Limiter,
    bridges: Limiter,
    accepts: Limiter,
    rooms: Mutex<HashMap<String, Room>>,
    next_epoch: AtomicU64,
}

/// The relay. It keeps everything in memory; restarting it just makes bridges reconnect.
#[derive(Clone)]
pub struct Relay(Arc<Inner>);

#[derive(Clone, Copy)]
enum Kind {
    Bridge,
    Client,
    Accept,
}

impl Relay {
    pub fn new(cfg: Config, log: LogFn) -> Relay {
        let minute = Duration::from_secs(60);
        Relay(Arc::new(Inner {
            clients: Limiter::new(cfg.clients_per_min, minute),
            bridges: Limiter::new(cfg.bridges_per_min, minute),
            accepts: Limiter::new(cfg.accepts_per_min, minute),
            cfg,
            log,
            rooms: Mutex::new(HashMap::new()),
            next_epoch: AtomicU64::new(1),
        }))
    }

    pub fn router(&self) -> Router {
        let health = self.clone();
        let mk = |me: &Relay, kind: Kind| {
            let me = me.clone();
            get(move |ci: ConnectInfo<SocketAddr>, req: Request| {
                let me = me.clone();
                async move { me.handle(kind, ci.0, req).await }
            })
        };
        Router::new()
            .route(
                "/health",
                get(move || async move {
                    let n = health.0.rooms.lock().unwrap().len();
                    ([(header::CONTENT_TYPE, "application/json")], format!("{{\"ok\":true,\"bridges\":{n}}}\n"))
                }),
            )
            .route(BRIDGE_PATH, mk(self, Kind::Bridge))
            .route(CLIENT_PATH, mk(self, Kind::Client))
            .route(ACCEPT_PATH, mk(self, Kind::Accept))
    }

    fn log(&self, s: &str) {
        (self.0.log)(s);
    }

    /// Applies a limit before anything is allocated for a connection. Over the limit: 429.
    fn admit(&self, kind: Kind, headers: &HeaderMap, peer: SocketAddr) -> Result<(), Response> {
        let l = match kind {
            Kind::Bridge => &self.0.bridges,
            Kind::Client => &self.0.clients,
            Kind::Accept => &self.0.accepts,
        };
        if l.allow(&source_of(headers, peer, self.0.cfg.trust_proxy), Instant::now()) {
            return Ok(());
        }
        Err((StatusCode::TOO_MANY_REQUESTS, [(header::RETRY_AFTER, "60")], "too many connections from this address\n").into_response())
    }

    async fn handle(&self, kind: Kind, peer: SocketAddr, req: Request<Body>) -> Response {
        if let Err(r) = self.admit(kind, req.headers(), peer) {
            return r;
        }
        let query = req.uri().query().unwrap_or("").to_string();
        let param = |name: &str| -> String { query.split('&').filter_map(|kv| kv.split_once('=')).find(|(k, _)| *k == name).map(|(_, v)| v.to_string()).unwrap_or_default() };
        let room = param("room");
        if !valid_room(&room) {
            return (StatusCode::BAD_REQUEST, "bad room\n").into_response();
        }
        let session = param("session");
        let upgrade = match WebSocketUpgrade::from_request(req, &()).await {
            Ok(u) => u,
            Err(e) => return e.into_response(),
        };
        let me = self.clone();
        upgrade.max_message_size(MAX_FRAME_BYTES).max_frame_size(MAX_FRAME_BYTES).on_upgrade(move |sock| async move {
            match kind {
                Kind::Bridge => me.bridge(sock, room).await,
                Kind::Client => me.client(sock, room).await,
                Kind::Accept => me.accept(sock, room, session).await,
            }
        })
    }

    /// Forgets a room with no bridge and no sessions.
    fn drop_if_idle(rooms: &mut HashMap<String, Room>, id: &str) {
        if rooms.get(id).is_some_and(|r| r.ctrl.is_none() && r.sessions == 0) {
            rooms.remove(id);
        }
    }

    /// Holds a bridge's control connection open; a newer one for the same room wins.
    async fn bridge(&self, mut sock: WebSocket, id: String) {
        let (tx, mut rx) = mpsc::channel::<Ctrl>(8);
        let epoch = self.0.next_epoch.fetch_add(1, Ordering::Relaxed);
        let full = {
            let mut rooms = self.0.rooms.lock().unwrap();
            if !rooms.contains_key(&id) && rooms.len() >= self.0.cfg.max_rooms {
                true
            } else {
                let cfg = &self.0.cfg;
                let rm = rooms.entry(id.clone()).or_insert_with(|| Room { bucket: Arc::new(ShareBucket::new(cfg.room_bytes_per_sec, cfg.room_burst, Instant::now())), ctrl: None, pending: HashMap::new(), sessions: 0 });
                if let Some(old) = rm.ctrl.replace(CtrlHandle { epoch, tx }) {
                    let _ = old.tx.try_send(Ctrl::Replaced);
                }
                false
            }
        };
        if full {
            self.log("relay full: refused a new room");
            close_with(&mut sock, STATUS_RELAY_FULL, "relay is full").await;
            return;
        }
        self.log(&format!("bridge up: room {}…", &id[..8]));
        // Reading keeps pings/pongs flowing and tells us when the bridge goes away.
        loop {
            tokio::select! {
                m = sock.recv() => match m {
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => {}
                },
                c = rx.recv() => match c {
                    Some(Ctrl::Session { frame, written }) => {
                        let ok = sock.send(Message::Text(frame.into())).await.is_ok();
                        let _ = written.send(ok);
                        if !ok { break; }
                    }
                    Some(Ctrl::Replaced) | None => {
                        close_with(&mut sock, STATUS_GOING_AWAY, "replaced").await;
                        break;
                    }
                },
            }
        }
        let mut rooms = self.0.rooms.lock().unwrap();
        if let Some(rm) = rooms.get_mut(&id) {
            if rm.ctrl.as_ref().is_some_and(|c| c.epoch == epoch) {
                rm.ctrl = None;
                Self::drop_if_idle(&mut rooms, &id);
            }
        }
        drop(rooms);
        self.log(&format!("bridge down: room {}…", &id[..8]));
    }

    /// Asks the room's bridge to open a session and waits until it is piped through.
    async fn client(&self, mut sock: WebSocket, id: String) {
        let sid = random_session_id();
        let (accepted_tx, accepted_rx) = oneshot::channel::<WebSocket>();
        enum Admit {
            Offline,
            Busy,
            Ok(mpsc::Sender<Ctrl>, Arc<ShareBucket>),
        }
        let admit = {
            let mut rooms = self.0.rooms.lock().unwrap();
            match rooms.get_mut(&id).filter(|r| r.ctrl.is_some()) {
                None => Admit::Offline,
                Some(rm) if rm.sessions >= MAX_SESSIONS_PER_ROOM => Admit::Busy,
                Some(rm) => {
                    rm.sessions += 1;
                    rm.pending.insert(sid.clone(), accepted_tx);
                    Admit::Ok(rm.ctrl.as_ref().map(|c| c.tx.clone()).unwrap(), rm.bucket.clone())
                }
            }
        };
        let (ctrl, bucket) = match admit {
            Admit::Offline => return close_with(&mut sock, STATUS_BRIDGE_OFFLINE, "bridge offline").await,
            Admit::Busy => return close_with(&mut sock, STATUS_ROOM_BUSY, "too many sessions").await,
            Admit::Ok(c, b) => (c, b),
        };
        let leave = |me: &Relay| {
            let mut rooms = me.0.rooms.lock().unwrap();
            if let Some(rm) = rooms.get_mut(&id) {
                rm.pending.remove(&sid);
                rm.sessions -= 1;
                Self::drop_if_idle(&mut rooms, &id);
            }
        };

        let frame = format!("{{\"type\":\"session\",\"id\":\"{sid}\"}}");
        let (written_tx, written_rx) = oneshot::channel();
        // both waits have a deadline: a bridge whose connection is dead without a reset must not
        // hold this plugin's slot until TCP gives up
        let sent = tokio::time::timeout(ACCEPT_TIMEOUT, ctrl.send(Ctrl::Session { frame, written: written_tx })).await.is_ok_and(|r| r.is_ok());
        let written = sent && tokio::time::timeout(ACCEPT_TIMEOUT, written_rx).await.is_ok_and(|r| r.unwrap_or(false));
        if !written {
            leave(self);
            close_with(&mut sock, STATUS_BRIDGE_OFFLINE, "bridge offline").await;
            return;
        }
        let mut accepted_rx = accepted_rx;
        let bridge_side = match tokio::time::timeout(ACCEPT_TIMEOUT, &mut accepted_rx).await {
            Ok(Ok(b)) => Some(b),
            Ok(Err(_)) => None, // the pending entry was dropped without an answer
            Err(_) => {
                let still_pending = {
                    let mut rooms = self.0.rooms.lock().unwrap();
                    rooms.get_mut(&id).is_some_and(|rm| rm.pending.remove(&sid).is_some())
                };
                if still_pending {
                    None
                } else {
                    accepted_rx.await.ok() // accepted at the last moment; wait for the socket
                }
            }
        };
        match bridge_side {
            Some(b) => pipe(sock, b, self.0.cfg.idle_timeout, Some(bucket)).await,
            None => close_with(&mut sock, STATUS_BRIDGE_OFFLINE, "bridge did not answer").await,
        }
        leave(self);
    }

    /// The bridge's per-session connection: it is handed to the waiting plugin's task, which pipes.
    async fn accept(&self, mut sock: WebSocket, id: String, session: String) {
        let waiting = {
            let mut rooms = self.0.rooms.lock().unwrap();
            rooms.get_mut(&id).and_then(|rm| rm.pending.remove(&session))
        };
        match waiting {
            None => close_with(&mut sock, STATUS_POLICY_VIOLATION, "unknown session").await,
            Some(tx) => {
                // A plugin that gave up has dropped its receiver: then the socket comes back as the error.
                if let Err(mut sock) = tx.send(sock) {
                    close_with(&mut sock, STATUS_POLICY_VIOLATION, "unknown session").await;
                }
            }
        }
    }
}

fn random_session_id() -> String {
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::rng().fill_bytes(&mut b);
    hex::encode(b)
}

async fn close_with(sock: &mut WebSocket, code: u16, reason: &'static str) {
    let _ = sock.send(Message::Close(Some(CloseFrame { code, reason: reason.into() }))).await;
}

/// Copies frames both ways until either side closes, the session has been silent for `idle`, or an
/// error ends it. The room's bucket slows a room that moves more than its share.
async fn pipe(a: WebSocket, b: WebSocket, idle: Duration, bucket: Option<Arc<ShareBucket>>) {
    let (mut a_tx, mut a_rx) = a.split();
    let (mut b_tx, mut b_rx) = b.split();
    loop {
        let (msg, to_b) = tokio::select! {
            m = a_rx.next() => (m, true),
            m = b_rx.next() => (m, false),
            _ = tokio::time::sleep(idle) => break,
        };
        let m = match msg {
            Some(Ok(m @ (Message::Text(_) | Message::Binary(_)))) => m,
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue, // each hop answers its own pings
            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
        };
        let n = match &m {
            Message::Text(t) => t.len(),
            Message::Binary(b) => b.len(),
            _ => 0,
        } as u64;
        let sent = if to_b { b_tx.send(m).await } else { a_tx.send(m).await };
        if sent.is_err() {
            break;
        }
        if let Some(bk) = &bucket {
            let wait = bk.take(n, Instant::now());
            if !wait.is_zero() {
                tokio::time::sleep(wait).await;
            }
        }
    }
    let normal = || Message::Close(Some(CloseFrame { code: STATUS_NORMAL, reason: "".into() }));
    let _ = a_tx.send(normal()).await;
    let _ = b_tx.send(normal()).await;
}

// ---- the command ----

pub async fn run_cli(flags: &HashMap<String, String>) -> Result<(), String> {
    let tls_domain = flags.get("tls-domain").cloned().filter(|d| !d.is_empty());
    let default_listen = if tls_domain.is_some() { ":8443" } else { ":8780" };
    let listen = flags.get("listen").filter(|l| !l.is_empty()).map(String::as_str).unwrap_or(default_listen);
    let listen = if let Some(port) = listen.strip_prefix(':') { format!("0.0.0.0:{port}") } else { listen.to_string() };
    let cfg = Config { trust_proxy: flags.contains_key("trust-proxy"), ..Config::default() };
    if tls_domain.is_some() && cfg.trust_proxy {
        return Err("--tls-domain and --trust-proxy do not go together: with its own TLS the relay sees the real address".into());
    }
    let log: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(|s| println!("{s}"));
    let relay = Relay::new(cfg, log.clone());
    let listener = tokio::net::TcpListener::bind(&listen).await.map_err(|e| format!("{listen}: {e}"))?;
    let cancel = tokio_util::sync::CancellationToken::new();
    crate::edge::cancel_on_signal(cancel.clone());
    let tls = match tls_domain {
        None => None,
        Some(domain) => Some(crate::names::service_tls(flags, &domain, "relay")?),
    };
    println!("spellink relay {} listening on {listen}{}", env!("CARGO_PKG_VERSION"), if tls.is_some() { " (TLS)" } else { "" });
    crate::edge::serve(listener, relay.router(), tls, cancel, log).await
}

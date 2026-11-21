//! The bridge's side of the relay (Go: internal/relay/connector.go): stay registered with it, and
//! serve every plugin session it announces.

use crate::relay::{room_id, ACCEPT_PATH, BRIDGE_PATH, CLIENT_PATH};
use crate::server::transport::Transport;
use crate::server::LogFn;
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

pub type Unit = Pin<Box<dyn Future<Output = ()> + Send>>;

/// Handles one plugin session arriving through the relay: the socket and who it is.
pub type ServeFn = Arc<dyn Fn(Transport, String) -> Unit + Send + Sync>;

/// Pings keep NAT mappings and proxies (e.g. Cloudflare's idle timeout) from dropping us.
const PING_INTERVAL: Duration = Duration::from_secs(30);

/// The address the plugin dials to reach the bridge through `relay_base` (e.g.
/// "wss://relay.example.com").
pub fn client_url(relay_base: &str, token: &str) -> String {
    format!("{}{CLIENT_PATH}?room={}", relay_base.trim_end_matches('/'), room_id(token))
}

/// Keeps the bridge registered with the relay until `cancel`, reconnecting with backoff, and
/// serves every session the relay announces.
pub async fn connect(cancel: CancellationToken, relay_base: String, token: String, serve: ServeFn, log: LogFn) {
    connect_room(cancel, relay_base, room_id(&token), serve, log).await;
}

/// `connect` for a room id that is already known (one device's secret gives one room).
pub async fn connect_room(cancel: CancellationToken, relay_base: String, room: String, serve: ServeFn, log: LogFn) {
    let base = relay_base.trim_end_matches('/').to_string();
    let mut backoff = Duration::from_secs(1);
    while !cancel.is_cancelled() {
        let started = Instant::now();
        let err = connect_once(&cancel, &base, &room, &serve, &log).await;
        if cancel.is_cancelled() {
            return;
        }
        if started.elapsed() > Duration::from_secs(60) {
            backoff = Duration::from_secs(1); // it was up for a while; retry quickly
        }
        log(&format!("relay: {err}; retrying in {}s", backoff.as_secs()));
        tokio::select! {
            _ = tokio::time::sleep(backoff) => {}
            _ = cancel.cancelled() => return,
        }
        backoff = (backoff * 2).min(Duration::from_secs(60));
    }
}

async fn connect_once(cancel: &CancellationToken, base: &str, room: &str, serve: &ServeFn, log: &LogFn) -> String {
    let url = format!("{base}{BRIDGE_PATH}?room={room}");
    let mut ws = tokio::select! {
        r = tokio_tungstenite::connect_async_with_config(url, None, true) => match r {
            Ok((ws, _)) => ws,
            Err(e) => return e.to_string(),
        },
        _ = cancel.cancelled() => return "cancelled".into(),
    };
    log(&format!("relay: connected to {base}"));
    // the relay answers pings, so a connection that has said nothing for two intervals is dead
    let mut ping = tokio::time::interval(PING_INTERVAL);
    ping.tick().await;
    let mut last_heard = Instant::now();
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return "cancelled".into(),
            _ = ping.tick() => {
                if last_heard.elapsed() > PING_INTERVAL * 2 + Duration::from_secs(10) {
                    return "the relay went silent".into();
                }
                if let Err(e) = ws.send(Message::Ping(Vec::new().into())).await {
                    return e.to_string();
                }
            }
            m = ws.next() => {
                last_heard = Instant::now();
                match m {
                    None => return "the relay closed the connection".into(),
                    Some(Err(e)) => return e.to_string(),
                    Some(Ok(Message::Close(f))) => return format!("the relay closed the connection: {f:?}"),
                    Some(Ok(Message::Text(t))) => {
                        let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) else { continue };
                        let (Some("session"), Some(sid)) = (v["type"].as_str(), v["id"].as_str().filter(|s| !s.is_empty())) else { continue };
                        let (u, serve, log, cancel) = (format!("{base}{ACCEPT_PATH}?room={room}&session={}", urlencode(sid)), serve.clone(), log.clone(), cancel.clone());
                        let sid = sid.to_string();
                        tokio::spawn(async move {
                            let sc = tokio::select! {
                                r = tokio_tungstenite::connect_async_with_config(u, None, true) => match r {
                                    Ok((sc, _)) => sc,
                                    Err(e) => return log(&format!("relay: accept {sid}: {e}")),
                                },
                                _ = cancel.cancelled() => return,
                            };
                            serve(Transport::from_tungstenite(sc), "relay".into()).await;
                        });
                    }
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}

fn urlencode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// Runs one room's registration until its token is cancelled (`connect_room`; a test replaces it).
pub type Runner = Arc<dyn Fn(CancellationToken, String, String, ServeFn, LogFn) -> Unit + Send + Sync>;

/// Keeps the bridge registered in several rooms at once and follows a set that changes as devices
/// are added, paired and revoked (ADR 0016: a device's B and K each have a room).
pub struct RoomSet {
    cancel: CancellationToken,
    base: String,
    log: LogFn,
    run: Runner,
    rooms: Mutex<HashMap<String, CancellationToken>>,
}

impl RoomSet {
    /// A set that registers with `relay_base` until `cancel`.
    pub fn new(cancel: CancellationToken, relay_base: &str, log: LogFn) -> RoomSet {
        let run: Runner = Arc::new(|c, base, room, serve, log| Box::pin(connect_room(c, base, room, serve, log)));
        RoomSet::with_runner(cancel, relay_base, log, run)
    }

    pub fn with_runner(cancel: CancellationToken, relay_base: &str, log: LogFn, run: Runner) -> RoomSet {
        RoomSet { cancel, base: relay_base.to_string(), log, run, rooms: Mutex::new(HashMap::new()) }
    }

    /// Makes the registered rooms exactly the keys of `want`: new rooms are registered, rooms that
    /// are no longer wanted are left. The serve function of a room that stays is not replaced.
    pub fn sync(&self, want: HashMap<String, ServeFn>) {
        let mut rooms = self.rooms.lock().unwrap();
        rooms.retain(|room, token| {
            let keep = want.contains_key(room);
            if !keep {
                token.cancel();
            }
            keep
        });
        for (room, serve) in want {
            if rooms.contains_key(&room) {
                continue;
            }
            let token = self.cancel.child_token();
            rooms.insert(room.clone(), token.clone());
            tokio::spawn((self.run)(token, self.base.clone(), room, serve, self.log.clone()));
        }
    }

    /// How many rooms are registered.
    pub fn len(&self) -> usize {
        self.rooms.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct RunLog {
        started: Mutex<Vec<String>>,
        stopped: Mutex<Vec<String>>,
    }

    fn runner(l: Arc<RunLog>) -> Runner {
        Arc::new(move |c, _base, room, _serve, _log| {
            let l = l.clone();
            Box::pin(async move {
                l.started.lock().unwrap().push(room.clone());
                c.cancelled().await;
                l.stopped.lock().unwrap().push(room);
            })
        })
    }

    async fn eventually(what: &str, f: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if f() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("timed out waiting for {what}");
    }

    fn serve() -> ServeFn {
        Arc::new(|_, _| Box::pin(async {}))
    }

    fn want(rooms: &[&str]) -> HashMap<String, ServeFn> {
        rooms.iter().map(|r| (r.to_string(), serve())).collect()
    }

    #[tokio::test]
    async fn the_room_set_follows_the_wanted_rooms() {
        let (l, cancel) = (Arc::new(RunLog::default()), CancellationToken::new());
        let rs = RoomSet::with_runner(cancel.clone(), "wss://relay.example", Arc::new(|_| {}), runner(l.clone()));
        rs.sync(want(&["a", "b"]));
        eventually("two rooms", || l.started.lock().unwrap().len() == 2).await;
        assert_eq!(rs.len(), 2);
        rs.sync(want(&["a", "c"])); // b leaves, c arrives, a stays untouched
        eventually("b stopped and c started", || l.started.lock().unwrap().len() == 3 && l.stopped.lock().unwrap().len() == 1).await;
        rs.sync(want(&["a", "c"])); // no change
        rs.sync(HashMap::new());
        eventually("all stopped", || l.stopped.lock().unwrap().len() == 3).await;
        assert!(rs.is_empty());
        assert_eq!(l.started.lock().unwrap().len(), 3, "an unchanged room was registered again");
    }

    #[tokio::test]
    async fn the_room_set_stops_everything_with_its_token() {
        let (l, cancel) = (Arc::new(RunLog::default()), CancellationToken::new());
        let rs = RoomSet::with_runner(cancel.clone(), "wss://relay.example", Arc::new(|_| {}), runner(l.clone()));
        rs.sync(want(&["a"]));
        eventually("started", || l.started.lock().unwrap().len() == 1).await;
        cancel.cancel();
        eventually("stopped by the parent token", || l.stopped.lock().unwrap().len() == 1).await;
    }

    #[tokio::test]
    async fn connecting_to_a_room_stops_at_once_when_already_cancelled() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(2), connect_room(cancel, "wss://relay.example".into(), room_id("x"), serve(), Arc::new(|_| {}))).await.expect("connect_room did not stop");
    }

    #[test]
    fn the_client_url_names_the_room_not_the_token() {
        let u = client_url("wss://relay.example/", "secret");
        assert_eq!(u, format!("wss://relay.example/v1/relay/client?room={}", room_id("secret")));
        assert!(!u.contains("secret"));
    }
}

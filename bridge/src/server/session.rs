//! One plugin session on an accepted WebSocket (Go: server/server.go and pairing.go).

use super::run::{Arg, DoneInfo, Run};
use super::run_table::{RunEvent, RunState};
use super::transport::{Incoming, Transport, Wire, WsMsg};
use super::{Deps, LogFn};
use crate::devices::registry::{Conn, DENY_UNKNOWN_DEVICE};
use crate::history::Turn;
use crate::layout::{Composer, Layout, capacity};
use crate::protocol::{self, ClientMessage, ErrorFrame, HistoryCleared, PairDenied, PairGrant, SettingsReply, Status, Welcome};
use crate::provider::{self, AskError, Request, Sink};
use crate::securechan::{self, Channel};
use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

pub const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
/// The largest WebSocket frame the plugin may send (a reassembled message may be larger, up to
/// `securechan::MAX_MESSAGE`).
pub const MAX_FRAME_BYTES: usize = 256 << 10;
pub const MAX_QUESTION_CHARS: usize = 4000;
pub const MAX_NOTE_KEY_CHARS: usize = 1024;
pub const MAX_PAGES: i64 = 10;
/// How long a pairing channel may stay silent before its pairRequest.
pub const DEFAULT_PAIR_REQUEST_WAIT: Duration = Duration::from_secs(10);
/// Bounds a pairing channel's whole life (the window and the grant's wait are shorter).
pub const DEFAULT_PAIRING_MAX: Duration = Duration::from_secs(300);

/// Why a read ended.
#[derive(Debug)]
pub enum ReadErr {
    /// The peer closed, or the connection went away: nothing to say back.
    Closed,
    /// What arrived was not acceptable: the peer is told (bad_request) and dropped.
    Bad(String),
    /// Nothing arrived in time.
    Timeout,
}

#[derive(Default)]
struct AskState {
    /// The answer this connection is waiting for, if any.
    current: Option<Arc<Run>>,
    /// Every run this connection received frames of, to detach on close.
    attached: Vec<Arc<Run>>,
}

pub struct Session {
    pub(super) deps: Arc<Deps>,
    pub peer: String,
    tx: mpsc::Sender<Wire>,
    expected: [u8; 32],
    require_encryption: bool,
    /// The device this session claims to be (empty: the shared token).
    pub device_id: String,
    /// Set once the Noise handshake succeeds.
    secure: OnceLock<Channel>,
    /// Opened with B: it may only pair.
    pairing: AtomicBool,
    /// The first failed write has been logged.
    write_err: AtomicBool,
    ask: Mutex<AskState>,
}

/// Checks an untrusted layout from the plugin.
pub fn valid_layout(l: &Layout) -> bool {
    let inr = |v: f64, lo: f64, hi: f64| v.is_finite() && v >= lo && v <= hi;
    const PAGE: f64 = 20000.0;
    inr(l.page_height, 100.0, PAGE)
        && inr(l.char_size, 12.0, 300.0)
        && inr(l.x, 0.0, PAGE)
        && inr(l.y, 0.0, l.page_height)
        && inr(l.width, l.char_size, PAGE)
        && inr(l.bottom_margin, 0.0, l.page_height)
        && (0..=MAX_PAGES).contains(&l.max_pages)
        && inr(l.continue_top, 0.0, l.page_height)
        && inr(l.start_limit, 0.0, l.page_height)
}

pub fn seed_from(id: &str) -> u32 {
    let h = Sha256::digest(id.as_bytes());
    u32::from_le_bytes([h[0], h[1], h[2], h[3]])
}

/// The answer length (in full-width characters) to ask the model for.
pub fn budget(l: &Layout) -> i64 {
    let mut b = capacity(l);
    if l.max_pages > 1 {
        b = b.max(200);
    }
    b.clamp(60, 600)
}

/// Compares two token hashes without stopping at the first difference.
fn constant_time_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn truncate_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

impl Session {
    /// Runs one session until it ends. `proxied` sessions (through a tunnel or the relay) only
    /// accept the encrypted handshake.
    pub async fn serve(deps: Arc<Deps>, transport: Transport, peer: &str, require_encryption: bool, device_id: &str) {
        let Transport { rx, tx } = transport;
        if deps.token.is_empty() && device_id.is_empty() {
            // (a device's session uses that device's own secrets, not this token)
            // the config never gives an empty token (it makes a new one); if one gets here anyway,
            // nobody is let in: the hash of "" would open both the plain and the encrypted way
            return;
        }
        let s = Arc::new(Session {
            expected: Sha256::digest(deps.token.as_bytes()).into(),
            deps,
            peer: peer.to_string(),
            tx,
            require_encryption,
            device_id: device_id.to_string(),
            secure: OnceLock::new(),
            pairing: AtomicBool::new(false),
            write_err: AtomicBool::new(false),
            ask: Mutex::new(AskState::default()),
        });
        s.run(rx).await;
    }

    /// A session with nothing attached to it, for the unit tests of the pieces that talk to one.
    #[cfg(test)]
    pub(super) fn detached_for_test(deps: Arc<Deps>) -> (Arc<Session>, mpsc::Receiver<Wire>) {
        let (tx, rx) = mpsc::channel(super::transport::QUEUE);
        let s = Arc::new(Session {
            expected: Sha256::digest(deps.token.as_bytes()).into(),
            deps,
            peer: "test".into(),
            tx,
            require_encryption: false,
            device_id: String::new(),
            secure: OnceLock::new(),
            pairing: AtomicBool::new(false),
            write_err: AtomicBool::new(false),
            ask: Mutex::new(AskState::default()),
        });
        (s, rx)
    }

    fn log(&self, s: &str) {
        (self.deps.log)(s);
    }

    pub fn send<T: Serialize>(&self, v: &T) {
        if let Ok(b) = serde_json::to_vec(v) {
            let _ = self.send_raw(&b);
        }
    }

    /// Writes one frame and reports whether it failed (a run turns that into SEND_FAIL).
    pub fn send_raw(&self, b: &[u8]) -> Result<(), String> {
        let r = match self.secure.get() {
            Some(ch) => ch.seal(b, |chunk| self.tx.try_send(Wire::Binary(chunk)).map_err(|_| securechan::Error::Cipher)).map_err(|_| "the connection is closed or too slow".to_string()),
            None => self.tx.try_send(Wire::Text(String::from_utf8_lossy(b).into_owned())).map_err(|_| "the connection is closed or too slow".to_string()),
        };
        // A failed write means the plugin did not get this frame; "done" in the log must not be
        // mistaken for "delivered" when the connection died underneath.
        if let Err(e) = &r {
            if !self.write_err.swap(true, Ordering::Relaxed) {
                self.log(&format!("write to {} failed: {e}", self.peer));
            }
        }
        r
    }

    fn provider_name(&self) -> String {
        self.deps.provider.name()
    }

    fn close_with_code(&self, code: u16, reason: &str) {
        let _ = self.tx.try_send(Wire::Close(code, reason.to_string()));
    }

    pub fn close_with(&self, reason: &str) {
        self.close_with_code(1008, reason);
    }

    fn fail(&self, code: &str, msg: &str) {
        self.send(&ErrorFrame::new("", code, msg));
        self.close_with(code);
    }

    /// Reads one frame (a whole message): from the encrypted channel, or as plain JSON text.
    async fn read(&self, rx: &mut Incoming) -> Result<ClientMessage, ReadErr> {
        let bytes = match self.secure.get() {
            Some(ch) => {
                let mut buf = Vec::new();
                loop {
                    match rx.next().await {
                        None | Some(Err(_)) => return Err(ReadErr::Closed),
                        Some(Ok(WsMsg::Text(_))) => return Err(ReadErr::Bad("expected a binary frame".into())),
                        Some(Ok(WsMsg::Binary(ct))) => match ch.open(&ct, &mut buf) {
                            Ok(true) => break buf,
                            Ok(false) => {}
                            Err(e) => return Err(ReadErr::Bad(e.to_string())),
                        },
                    }
                }
            }
            None => match rx.next().await {
                None | Some(Err(_)) => return Err(ReadErr::Closed),
                Some(Ok(WsMsg::Binary(_))) => return Err(ReadErr::Bad("frames must be JSON text".into())),
                Some(Ok(WsMsg::Text(t))) => t.into_bytes(),
            },
        };
        ClientMessage::parse(&bytes).map_err(|_| ReadErr::Bad("frames must be JSON".into()))
    }

    async fn read_within(&self, rx: &mut Incoming, d: Duration) -> Result<ClientMessage, ReadErr> {
        match tokio::time::timeout(d, self.read(rx)).await {
            Ok(r) => r,
            Err(_) => Err(ReadErr::Timeout),
        }
    }

    fn reject(&self, why: &str) -> Option<String> {
        self.log(&format!("rejected device connection from {}: {why}", self.peer));
        self.close_with("unauthorized");
        None
    }

    /// Accepts the Noise handshake with `psk`, answering with message 2.
    fn accept_noise(&self, psk: &[u8; 32], first: &[u8]) -> Result<(), securechan::Error> {
        let (ch, msg2) = Channel::accept(psk, first)?;
        self.tx.try_send(Wire::Binary(msg2)).map_err(|_| securechan::Error::Handshake)?;
        self.secure.set(ch).map_err(|_| securechan::Error::Handshake)
    }

    /// Handles the first frame: a binary Noise handshake (authenticated by the token-derived key),
    /// or — on the LAN only — a plaintext hello carrying the token.
    async fn authenticate(self: &Arc<Self>, rx: &mut Incoming) -> Option<String> {
        let first = match tokio::time::timeout(HELLO_TIMEOUT, rx.next()).await {
            Ok(Some(Ok(m))) => m,
            _ => return None,
        };
        if !self.device_id.is_empty() {
            return self.authenticate_device(first);
        }
        match first {
            WsMsg::Binary(b) => match self.accept_noise(&securechan::psk(&self.deps.token), &b) {
                Ok(()) => Some("noise".into()),
                Err(e) => {
                    self.log(&format!("rejected connection from {}: handshake: {e}", self.peer));
                    self.close_with("unauthorized");
                    None
                }
            },
            WsMsg::Text(t) => {
                let hello = ClientMessage::parse(t.as_bytes()).ok().filter(|h| h.kind == "hello");
                let Some(hello) = hello.filter(|_| !self.require_encryption) else {
                    self.fail("unauthorized", "encrypted handshake expected");
                    return None;
                };
                let got: [u8; 32] = Sha256::digest(hello.token.as_bytes()).into();
                if !constant_time_eq(&got, &self.expected) {
                    self.log(&format!("rejected connection from {}", self.peer));
                    self.fail("unauthorized", "bad token");
                    return None;
                }
                Some(format!("{} (plaintext)", hello.client))
            }
        }
    }

    /// Accepts only the Noise handshake, with the device's K (a normal session) or B (the pairing
    /// channel). K is tried first: a device that has it never needs B again.
    fn authenticate_device(self: &Arc<Self>, first: WsMsg) -> Option<String> {
        let (Some(devices), WsMsg::Binary(first)) = (self.deps.devices.clone(), first) else {
            return self.reject("a device must open with the encrypted handshake");
        };
        let Some((b, k)) = devices.secrets(&self.device_id) else {
            return self.reject("unknown or finished device");
        };
        if !k.is_empty() && self.accept_noise(&securechan::psk(&k), &first).is_ok() {
            if let Err(e) = devices.connect_k(&self.device_id) {
                return self.reject(&e.to_string());
            }
            self.deps.sessions.add(&self.device_id, self);
            return Some(format!("device {}", self.device_id));
        }
        if !b.is_empty() && self.accept_noise(&securechan::psk(&b), &first).is_ok() {
            self.pairing.store(true, Ordering::Relaxed);
            self.deps.sessions.add(&self.device_id, self);
            return Some(format!("pairing {}", self.device_id));
        }
        self.reject("handshake failed")
    }

    async fn run(self: Arc<Self>, mut rx: Incoming) {
        let client = self.authenticate(&mut rx).await;
        let Some(client) = client else {
            self.close_with_code(1000, "");
            return;
        };
        let device = !self.device_id.is_empty();
        if self.pairing.load(Ordering::Relaxed) {
            self.log(&format!("pairing channel: {} device {}", self.peer, self.device_id));
            self.run_pairing(&mut rx).await;
            if let Some(d) = &self.deps.devices {
                d.disconnect(&(self.clone() as Arc<dyn Conn>));
            }
        } else {
            self.log(&format!("connected: {} ({:.80})", self.peer, client));
            self.send(&Welcome { kind: "welcome", bridge: self.deps.name.clone(), provider: self.provider_name(), version: self.deps.version.clone(), protocol: protocol::VERSION, settings: (self.deps.settings)() });
            self.serve_messages(&mut rx).await;
        }
        if device {
            self.deps.sessions.remove(&self.device_id, &self);
        }
        self.detach_all();
        self.close_with_code(1000, "");
    }

    async fn serve_messages(self: &Arc<Self>, rx: &mut Incoming) {
        loop {
            let m = match self.read(rx).await {
                Ok(m) => m,
                Err(e) => {
                    self.log(&format!("disconnected: {}: {e:?}", self.peer));
                    if let ReadErr::Bad(msg) = e {
                        self.fail("bad_request", &msg);
                    }
                    return; // the answer being produced stays: the plugin may come back for it
                }
            };
            match m.kind.as_str() {
                "ask" => self.start_ask(m),
                "cancel" => {
                    // NoRun × CANCEL: an id nobody knows is ignored (docs/spec/bridge-run.md)
                    if let Some(r) = self.deps.runs.get(&m.id) {
                        r.fire(RunEvent::Cancel, Arg::None);
                    }
                }
                "resume" => self.resume(&m.id, m.next),
                "clearHistory" => {
                    // R9 (docs/spec/bridge-run.md): whatever runs exist, the memory is cleared
                    let n = self.deps.history.clear().unwrap_or_else(|e| {
                        self.log(&format!("history: clear: {e}"));
                        0
                    });
                    self.send(&HistoryCleared { kind: "historyCleared", notes: n });
                }
                "forgetNote" => {
                    // One note only. It is a message of its own — not clearHistory with a field — so
                    // a bridge that does not know it refuses it, instead of ignoring the field and
                    // forgetting every note.
                    if m.note.is_empty() || m.note.chars().count() > MAX_NOTE_KEY_CHARS {
                        self.fail("bad_request", "forgetNote needs the note's key");
                        return;
                    }
                    let n = self.deps.history.clear_note(&m.note).unwrap_or_else(|e| {
                        self.log(&format!("history: forget note: {e}"));
                        0
                    });
                    self.send(&HistoryCleared { kind: "historyCleared", notes: n });
                }
                "report" => (self.deps.report)(&self.peer, m.data.as_ref()),
                "setSettings" => {
                    let Some(settings) = m.settings else {
                        self.fail("bad_request", "settings missing");
                        return;
                    };
                    if let Err(e) = (self.deps.set_settings)(settings) {
                        self.log(&format!("settings: {e}"));
                    }
                    self.send(&SettingsReply { kind: "settings", settings: (self.deps.settings)() });
                }
                _ => {
                    self.fail("bad_request", "unknown message type");
                    return;
                }
            }
        }
    }

    /// Called when the connection ends: its answers keep being produced for a while so the plugin
    /// can reconnect and fetch them (ADR 0013).
    fn detach_all(self: &Arc<Self>) {
        let runs = std::mem::take(&mut self.ask.lock().unwrap().attached);
        for r in runs {
            r.disconnect(self);
        }
    }

    fn reply_to(self: &Arc<Self>, id: &str) -> Arg {
        Arg::Reply { session: self.clone(), id: id.to_string() }
    }

    fn start_ask(self: &Arc<Self>, m: ClientMessage) {
        let id = truncate_chars_bytes(&m.id, 64);
        let q = m.question.trim().to_string();
        let layout = m.layout.clone().filter(valid_layout);
        let Some(layout) = layout.filter(|_| !id.is_empty() && !q.is_empty() && q.chars().count() <= MAX_QUESTION_CHARS) else {
            self.send(&ErrorFrame::new(&id, "bad_request", "invalid ask"));
            return;
        };
        let lang = if m.lang == "en" { "en" } else { "ja" };

        let mut ask = self.ask.lock().unwrap();
        if let Some(cur) = ask.current.clone().filter(|c| !c.is_finished()) {
            drop(ask);
            cur.fire(RunEvent::AskWhileBusy, self.reply_to(&id)); // E3
            return;
        }
        // The answer must not die with this connection: a dropped tunnel would otherwise lose it.
        // It stops on an explicit cancel, or when nobody comes back for it (the run's grace).
        let cancel = tokio_util::sync::CancellationToken::new();
        let run = match self.deps.runs.start(&id, cancel.clone(), self.deps.log.clone(), self.deps.orphan_grace) {
            Ok(r) => r,
            Err(existing) => {
                drop(ask);
                cancel.cancel();
                existing.fire(RunEvent::AskRepeats, self.reply_to(&id)); // E2
                return;
            }
        };
        ask.current = Some(run.clone());
        ask.attached.push(run.clone());
        drop(ask);
        run.attach_first(self);

        // While the answer is being produced, say so: the plugin takes "nothing at all for a while"
        // to mean the connection is dead.
        let every = if self.deps.keepalive_every.is_zero() { Duration::from_secs(5) } else { self.deps.keepalive_every };
        let (r2, c2) = (run.clone(), cancel.clone());
        tokio::spawn(async move {
            let mut t = tokio::time::interval(every);
            t.tick().await; // the first tick is immediate
            loop {
                tokio::select! {
                    _ = c2.cancelled() => return,
                    _ = t.tick() => r2.fire(RunEvent::KeepaliveTick, Arg::None),
                }
            }
        });

        let (s, note) = (self.clone(), m.note.clone());
        tokio::spawn(async move {
            s.answer(&run, &id, &q, lang, &note, layout).await;
            if !run.is_finished() {
                // an answer must end in a frame, whatever happened
                run.fire(RunEvent::AiFails, Arg::Fail { code: "provider_failed".into(), message: "the answer ended without a result".into() });
            }
            run.cancel.cancel();
            let mut ask = s.ask.lock().unwrap();
            if ask.current.as_ref().is_some_and(|c| Arc::ptr_eq(c, &run)) {
                ask.current = None;
            }
        });
    }

    /// Hands a reconnected plugin the frames of answer `id` from number `next` on, and keeps
    /// sending the rest live.
    fn resume(self: &Arc<Self>, id: &str, next: i64) {
        let Some(r) = self.deps.runs.get(id) else {
            // NoRun × RESUME (docs/spec/bridge-run.md): the answer is not kept.
            if matches!(super::run::rule_n(RunEvent::DeviceResumes), super::run_table::Cell::Do { action: super::run_table::RunAction::E1, .. }) {
                self.send(&ErrorFrame::new(id, "unknown_ask", "this answer is no longer kept"));
            }
            return;
        };
        {
            let mut ask = self.ask.lock().unwrap();
            ask.attached.push(r.clone());
            if !r.is_finished() {
                ask.current = Some(r.clone());
            }
        }
        self.log(&format!("resume {id} from frame {next} for {}", self.peer));
        r.fire(RunEvent::DeviceResumes, Arg::Resume { session: self.clone(), next });
    }

    async fn answer(self: &Arc<Self>, r: &Arc<Run>, id: &str, q: &str, lang: &str, note: &str, mut layout: Layout) {
        let started = Instant::now();
        let st = (self.deps.settings)();
        if st.line_tilt {
            // Positive degrees rise to the right, i.e. y decreases as x grows.
            layout.slope = -(st.tilt_degrees * std::f64::consts::PI / 180.0).tan();
        }
        layout.spacing_full = st.full_width_spacing as f64 / 100.0;
        layout.spacing_half = st.half_width_spacing as f64 / 100.0;
        let Some(font) = self.deps.font.clone() else {
            r.fire(RunEvent::AiFails, Arg::Fail { code: "provider_failed".into(), message: "no handwriting data".into() });
            return;
        };
        let budget = budget(&layout);
        let composer = Arc::new(Mutex::new(Composer::new(font, layout, seed_from(id))));
        let emit = {
            let r = r.clone();
            move |lines: Vec<crate::layout::Line>| {
                if !lines.is_empty() {
                    r.fire(RunEvent::TextArrives, Arg::Lines(lines));
                }
            }
        };
        self.log(&format!("ask {id}: {}", truncate_chars(&q.split_whitespace().collect::<Vec<_>>().join(" "), 80)));
        r.notify(&Status { kind: "status", id: id.to_string(), stage: "thinking", phase: None, idle_ms: None });

        let note = if note.chars().count() > MAX_NOTE_KEY_CHARS { "" } else { note }; // not a real note key: ask without memory rather than guess
        let earlier = self.deps.history.recent(note, self.deps.history_chars).unwrap_or_else(|e| {
            self.log(&format!("history: read: {e}")); // the question is still worth answering
            vec![]
        });

        // The AI must show it is working: the model has to begin soon after the process starts,
        // then never be silent for long — thinking counts, only nothing at all does — all within a
        // total (bridge-run spec: HANG). Whichever limit hits first stops it.
        let or = |d: Duration, def: u64| if d.is_zero() { Duration::from_secs(def) } else { d };
        let (first, gap, total) = (or(self.deps.first_output, 10), or(self.deps.gap, 10), or(self.deps.total, 30));
        let timers = Arc::new(Timers::new(r.clone(), gap));
        timers.set_total(total, format!("the AI did not finish within {} s", total.as_secs()));
        timers.set_first(first, format!("the AI showed no sign of working within {} s", first.as_secs()));
        let gap_why = format!("the AI stopped working for more than {} s", gap.as_secs());

        let on_activity = {
            let (r, timers, log, id, gap_why) = (r.clone(), timers.clone(), self.deps.log.clone(), id.to_string(), gap_why);
            move |model: bool| {
                let mut began = timers.began.lock().unwrap();
                if !model && !*began {
                    r.sign(""); // the process runs; the model has not begun, the first limit keeps counting
                    return;
                }
                if !*began {
                    *began = true;
                    timers.stop_first();
                    timers.set_gap(&gap_why);
                    r.sign("working");
                    log(&format!("ask {id} model began after {} ms", started.elapsed().as_millis()));
                    return;
                }
                r.sign("");
                if !r.is_finished() {
                    timers.set_gap(&gap_why); // a late sign must not revive an answer that already ended
                }
            }
        };
        let writing = Arc::new(AtomicBool::new(false));
        let on_text = {
            let (r, composer, writing, log, id, activity) = (r.clone(), composer.clone(), writing.clone(), self.deps.log.clone(), id.to_string(), on_activity.clone());
            move |delta: &str| {
                activity(true);
                if !writing.swap(true, Ordering::Relaxed) {
                    r.sign("writing");
                    log(&format!("ask {id} first output after {} ms", started.elapsed().as_millis()));
                    r.notify(&Status { kind: "status", id: id.clone(), stage: "writing", phase: None, idle_ms: None });
                }
                let (lines, truncated) = {
                    let mut c = composer.lock().unwrap();
                    (c.push(delta), c.truncated)
                };
                emit(lines);
                if truncated {
                    r.cancel.cancel(); // stop paying for tokens that can no longer fit
                }
            }
        };

        let result = self
            .deps
            .provider
            .ask(
                Request { system: provider::system_prompt(lang, budget), earlier, question: q.to_string(), lang: lang.to_string() },
                Sink {
                    text: Arc::new(on_text),
                    activity: Arc::new(on_activity),
                    tool: Arc::new({
                        let (log, id) = (self.deps.log.clone(), id.to_string());
                        move |l: &str| log(&format!("ask {id} {l}"))
                    }),
                },
                r.cancel.clone(),
            )
            .await;
        timers.stop_all();

        let truncated_now = composer.lock().unwrap().truncated;
        let text = match result {
            Err(e) if !truncated_now => {
                let (mut code, mut msg) = match &e {
                    AskError::Cancelled(_) => ("provider_failed", e.to_string()),
                    AskError::Failed(m) => ("provider_failed", m.clone()),
                };
                if r.cancel.is_cancelled() {
                    (code, msg) = ("cancelled", "cancelled".to_string());
                }
                let msg = truncate_chars_bytes(&msg, 500);
                self.log(&format!("ask {id} {code}: {msg}"));
                // A run that already ended (cancelled, timed out) ignores this late result:
                // AiFails in a finished state is an "ignore" cell.
                r.fire(RunEvent::AiFails, Arg::Fail { code: code.into(), message: msg });
                return;
            }
            Ok(t) => t,
            Err(AskError::Cancelled(t)) => t, // cut short on purpose: the page is full
            Err(AskError::Failed(_)) => String::new(),
        };
        let (lines, missing, truncated, pages) = {
            let mut c = composer.lock().unwrap();
            let lines = c.finish();
            let mut missing: Vec<String> = c.missing.iter().map(|m| m.to_string()).collect();
            missing.sort();
            (lines, missing, c.truncated, c.pages)
        };
        if !lines.is_empty() {
            r.fire(RunEvent::TextArrives, Arg::Lines(lines));
        }
        let elapsed = started.elapsed().as_millis() as i64;
        self.log(&format!("ask {id} done in {elapsed} ms, {pages} page(s){}", if truncated { " (truncated)" } else { "" }));
        let remember: Box<dyn FnOnce() + Send> = {
            let (s, note, q, text) = (self.clone(), note.to_string(), q.to_string(), text.clone());
            Box::new(move || {
                if s.deps.history_chars > 0 && !text.trim().is_empty() {
                    let turn = Turn { at: crate::util::now_rfc3339(), q, a: text };
                    if let Err(e) = s.deps.history.append(&note, &turn) {
                        s.log(&format!("history: write: {e}"));
                    }
                }
            })
        };
        let info = DoneInfo { text, missing, truncated, pages, elapsed_ms: elapsed, remember: Some(remember) };
        r.fire(if truncated { RunEvent::PagesRunOut } else { RunEvent::AiFinishes }, Arg::Done(info));
    }

    // ---- the pairing channel (ADR 0016) ----

    /// Serves the pairing channel until it is granted, refused or the device leaves. A device
    /// session opened with B may send one pairRequest and, after it was granted, one pairAck.
    /// Anything else — an ask above all — ends it.
    async fn run_pairing(self: &Arc<Self>, rx: &mut Incoming) {
        let Some(devices) = self.deps.devices.clone() else { return };
        let max = if self.deps.pairing_max.is_zero() { DEFAULT_PAIRING_MAX } else { self.deps.pairing_max };
        let wait = if self.deps.pair_request_wait.is_zero() { DEFAULT_PAIR_REQUEST_WAIT } else { self.deps.pair_request_wait };
        let deadline = tokio::time::Instant::now() + max;
        let mut requested = false;
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            let limit = if requested { left } else { left.min(wait) };
            let m = match self.read_within(rx, limit).await {
                Ok(m) => m,
                Err(e) => {
                    self.log(&format!("pairing channel {} closed: {e:?}", self.peer));
                    return;
                }
            };
            match m.kind.as_str() {
                "pairRequest" => {
                    if requested {
                        self.fail("bad_request", "one pairRequest per channel");
                        return;
                    }
                    if m.id != self.device_id {
                        // a channel opened for one device must not pair another
                        self.fail("bad_request", "the request is for another device");
                        return;
                    }
                    requested = true;
                    if let Err(e) = devices.request(&m.id, m.generation, &m.code, self.clone()) {
                        // the ledger has told the device (Deny) and closed the channel
                        self.log(&format!("pairRequest from {} refused: {e}", self.peer));
                        return;
                    }
                }
                "pairAck" => {
                    if m.id != self.device_id {
                        self.fail("bad_request", "the ack is for another device");
                        return;
                    }
                    if let Err(e) = devices.ack(&m.id, m.generation) {
                        self.log(&format!("pairAck from {} refused: {e}", self.peer));
                    }
                    return; // the device reconnects with K
                }
                _ => {
                    self.fail("bad_request", "only pairing is possible on this channel");
                    return;
                }
            }
        }
    }
}

/// The ledger tells a waiting device it is granted K, or refused.
impl Conn for Session {
    /// Sends K inside the encrypted channel.
    fn grant(&self, key: &str, endpoints: &[String]) -> Result<(), String> {
        let b = serde_json::to_vec(&PairGrant { kind: "pairGrant", key: key.to_string(), endpoints: endpoints.to_vec() }).map_err(|e| e.to_string())?;
        self.send_raw(&b)
    }

    /// Says why and closes the channel.
    fn deny(&self, reason: &str) {
        self.send(&PairDenied { kind: "pairDenied", reason: reason.to_string() });
        self.close_with("pair_denied");
    }
}

fn truncate_chars_bytes(s: &str, max_bytes: usize) -> String {
    // Go cuts an id or a message at a byte count; keep it a valid string
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// The AI's silence limits for one answer: a total, the first sign of work, then the gap between
/// signs once it works. Each is a task that fires AiGoesSilent unless it is stopped or replaced.
struct Timers {
    run: Arc<Run>,
    gap: Duration,
    total: Mutex<Option<AbortHandle>>,
    first: Mutex<Option<AbortHandle>>,
    gap_timer: Mutex<Option<AbortHandle>>,
    /// The model has shown its first sign.
    began: Mutex<bool>,
}

impl Timers {
    fn new(run: Arc<Run>, gap: Duration) -> Timers {
        Timers { run, gap, total: Mutex::new(None), first: Mutex::new(None), gap_timer: Mutex::new(None), began: Mutex::new(false) }
    }

    fn arm(&self, slot: &Mutex<Option<AbortHandle>>, after: Duration, why: String) {
        let run = self.run.clone();
        let h = tokio::spawn(async move {
            tokio::time::sleep(after).await;
            run.fire(RunEvent::AiGoesSilent, Arg::Why(why));
        });
        if let Some(old) = slot.lock().unwrap().replace(h.abort_handle()) {
            old.abort();
        }
    }

    fn set_total(&self, d: Duration, why: String) {
        self.arm(&self.total, d, why);
    }
    fn set_first(&self, d: Duration, why: String) {
        self.arm(&self.first, d, why);
    }
    fn set_gap(&self, why: &str) {
        self.arm(&self.gap_timer, self.gap, why.to_string());
    }
    fn stop_first(&self) {
        if let Some(h) = self.first.lock().unwrap().take() {
            h.abort();
        }
    }
    fn stop_all(&self) {
        for slot in [&self.total, &self.first, &self.gap_timer] {
            if let Some(h) = slot.lock().unwrap().take() {
                h.abort();
            }
        }
    }
}

/// The live sessions of each device, so that revoking a device cuts them off.
#[derive(Default)]
pub struct Sessions {
    by_id: Mutex<HashMap<String, Vec<Arc<Session>>>>,
}

impl Sessions {
    pub fn add(&self, id: &str, s: &Arc<Session>) {
        self.by_id.lock().unwrap().entry(id.to_string()).or_default().push(s.clone());
    }

    pub fn remove(&self, id: &str, s: &Arc<Session>) {
        let mut m = self.by_id.lock().unwrap();
        if let Some(v) = m.get_mut(id) {
            v.retain(|x| !Arc::ptr_eq(x, s));
            if v.is_empty() {
                m.remove(id);
            }
        }
    }

    /// Ends every session of the device (it was revoked or reissued).
    pub fn close_device(&self, id: &str) {
        let live: Vec<Arc<Session>> = self.by_id.lock().unwrap().get(id).cloned().unwrap_or_default();
        for s in live {
            s.close_with("device_removed");
        }
    }

    pub fn count(&self, id: &str) -> usize {
        self.by_id.lock().unwrap().get(id).map_or(0, |v| v.len())
    }
}

// keep these names used even where the build has no consumer yet
const _: &str = DENY_UNKNOWN_DEVICE;
const _: Option<RunState> = None;
const _: Option<LogFn> = None;

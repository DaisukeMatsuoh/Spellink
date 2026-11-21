//! A run is one answer, from the question until it has been delivered (Go: server/runs.go). It
//! outlives the WebSocket that asked (ADR 0013): the frames it produces are numbered and kept, so a
//! plugin whose connection dropped can reconnect, say "I have everything before frame N" and get
//! the rest. Nothing is guessed or retried: the plugin names the id and the number it is missing.
//!
//! What happens when is the table in docs/spec/bridge-run.md (run_table.rs, generated). An event is
//! fired with `Run::fire`; the cell names the action and the next state; the handlers below are the
//! actions. Every handler runs with the run's lock held.

use super::run_table::{cell, Cell, RunAction, RunEvent, RunMove, RunState};
use super::session::Session;
use super::LogFn;
use crate::layout::Line as InkLine;
use crate::protocol::{Done, ErrorFrame, LineFrame, Status};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// How long an answer keeps being produced with no plugin connected.
pub const DEFAULT_ORPHAN_GRACE: Duration = Duration::from_secs(30);

/// Completes an answer; `remember` is called once, to keep the exchange in the note's
/// conversation memory.
pub struct DoneInfo {
    pub text: String,
    pub missing: Vec<String>,
    pub truncated: bool,
    pub pages: i64,
    pub elapsed_ms: i64,
    pub remember: Option<Box<dyn FnOnce() + Send>>,
}

/// The payloads of events.
pub enum Arg {
    None,
    /// DeviceResumes: replay from `next`, then `session` receives.
    Resume { session: Arc<Session>, next: i64 },
    /// What the error actions (E1–E3) answer to.
    Reply { session: Arc<Session>, id: String },
    Lines(Vec<InkLine>),
    Done(DoneInfo),
    Fail { code: String, message: String },
    /// AiGoesSilent: why.
    Why(String),
    /// SendFails: the write error.
    SendErr(String),
    /// DeviceLeaves: the session that left.
    Left(Arc<Session>),
}

/// Where the AI is, and when it last showed a sign of life.
struct Vitals {
    phase: &'static str,
    last: Instant,
}

fn phase_rank(p: &str) -> u8 {
    match p {
        "working" => 1,
        "writing" => 2,
        _ => 0, // starting
    }
}

struct Inner {
    state: RunState,
    /// line / done / error frames, in order; frame i carries "seq": i
    frames: Vec<Vec<u8>>,
    /// When the answer was completed (for the retention).
    finish_at: Option<Instant>,
    /// Where new frames go now; none while the plugin is disconnected.
    to: Option<Arc<Session>>,
    orphan: Option<JoinHandle<()>>,
    /// The first failed write has been logged.
    send_err: bool,
}

pub struct Run {
    pub id: String,
    /// Stops the AI.
    pub cancel: CancellationToken,
    log: LogFn,
    grace: Duration,
    inner: Mutex<Inner>,
    vitals: Mutex<Vitals>,
}

fn err_frame(id: &str, seq: i64, code: &str, message: &str) -> ErrorFrame {
    ErrorFrame { kind: "error", id: id.to_string(), seq: Some(seq), code: code.to_string(), message: message.to_string() }
}

impl Run {
    pub fn new(id: &str, cancel: CancellationToken, start: RunState, log: LogFn, grace: Duration) -> Arc<Run> {
        Arc::new(Run {
            id: id.to_string(),
            cancel,
            log,
            grace: if grace.is_zero() { DEFAULT_ORPHAN_GRACE } else { grace },
            inner: Mutex::new(Inner { state: start, frames: vec![], finish_at: None, to: None, orphan: None, send_err: false }),
            vitals: Mutex::new(Vitals { phase: "starting", last: Instant::now() }),
        })
    }

    /// Reports an event to the run's table.
    pub fn fire(self: &Arc<Self>, ev: RunEvent, arg: Arg) {
        let mut inner = self.inner.lock().unwrap();
        self.dispatch(&mut inner, ev, arg, false);
    }

    pub fn state(&self) -> RunState {
        self.inner.lock().unwrap().state
    }

    pub fn is_finished(&self) -> bool {
        matches!(self.state(), RunState::DoneLive | RunState::DoneAway)
    }

    pub fn frame_count(&self) -> usize {
        self.inner.lock().unwrap().frames.len()
    }

    /// Tells the run that session `s`'s connection ended; only the session that is receiving it
    /// matters (one already replaced by a newer connection is not news).
    pub fn disconnect(self: &Arc<Self>, s: &Arc<Session>) {
        let mut inner = self.inner.lock().unwrap();
        if inner.to.as_ref().is_some_and(|t| Arc::ptr_eq(t, s)) {
            self.dispatch(&mut inner, RunEvent::DeviceLeaves, Arg::Left(s.clone()), false);
        }
    }

    /// Makes `s` the receiver of a new run (R1: the first connection of the answer).
    pub fn attach_first(&self, s: &Arc<Session>) {
        self.inner.lock().unwrap().to = Some(s.clone());
    }

    /// Looks up the cell for (state, event), moves, and calls the action. The only place that
    /// changes `state`.
    fn dispatch(self: &Arc<Self>, inner: &mut Inner, ev: RunEvent, arg: Arg, spent: bool) {
        match cell(inner.state, ev) {
            // The table says this cannot happen here. It did: record it loudly, change nothing.
            Cell::Never(note) => (self.log)(&format!("run.never: state {} event {} (note {note})", inner.state.name(), ev.name())),
            Cell::Ignore(_) => {}
            Cell::IgnoreDo(a) => self.act(inner, a, arg),
            Cell::Do { action, next } => self.mv(inner, RunMove { action, next }, arg),
            Cell::Guard { more, spent: s } => self.mv(inner, if spent { s } else { more }, arg),
        }
    }

    fn mv(self: &Arc<Self>, inner: &mut Inner, m: RunMove, arg: Arg) {
        inner.state = m.next; // first, so an action that fires further events sees the new state
        self.act(inner, m.action, arg);
    }

    fn act(self: &Arc<Self>, inner: &mut Inner, action: RunAction, arg: Arg) {
        use RunAction::*;
        match action {
            R1 => {} // the run was created by the registry (Runs::start)
            R2 => {
                // CANCEL
                self.cancel.cancel();
                self.publish(inner, |id, seq| err_frame(id, seq, "cancelled", "cancelled"));
            }
            R3 => {
                // DISCONNECT while the AI is working: wait a while for the plugin
                inner.to = None;
                if inner.orphan.is_none() {
                    let (run, grace) = (self.clone(), self.grace);
                    inner.orphan = spawn(async move {
                        tokio::time::sleep(grace).await;
                        run.fire(RunEvent::GraceEnds, Arg::None);
                    });
                }
            }
            R4 => {
                // RESUME: replay from `next`, then the newest connection receives
                let Arg::Resume { session, next } = arg else { unreachable!("RESUME carries the session") };
                self.stop_orphan(inner);
                let from = next.max(0) as usize;
                for i in from..inner.frames.len() {
                    let f = inner.frames[i].clone();
                    self.send_locked(inner, &session, &f);
                }
                inner.to = Some(session);
            }
            R5 => {
                // DELTA
                let Arg::Lines(lines) = arg else { unreachable!() };
                for l in lines {
                    self.publish(inner, move |id, seq| LineFrame { kind: "line", id: id.to_string(), seq, line: l });
                }
            }
            R6 => {
                // GRACE: nobody came back
                self.cancel.cancel();
                self.publish(inner, |id, seq| err_frame(id, seq, "cancelled", "cancelled"));
            }
            R7 => inner.to = None, // DISCONNECT after the answer is complete
            R8 => {
                let Arg::Done(d) = arg else { unreachable!() };
                self.complete(inner, d);
            }
            R9 => {} // CLEAR: the memory is cleared by the session, whatever runs exist
            R10 => {
                // PROVIDER_FAIL
                let Arg::Fail { code, message } = arg else { unreachable!() };
                self.publish(inner, move |id, seq| err_frame(id, seq, &code, &message));
                inner.finish_at = Some(Instant::now());
            }
            R11 => {
                // FULL: no more pages; stop paying for text that cannot be written
                self.cancel.cancel();
                let Arg::Done(mut d) = arg else { unreachable!() };
                d.truncated = true;
                self.complete(inner, d);
            }
            R12 => {
                // SEND_FAIL: the frames are kept, so a reconnect gets them
                if !inner.send_err {
                    inner.send_err = true;
                    let why = match arg {
                        Arg::SendErr(e) => e,
                        _ => String::new(),
                    };
                    (self.log)(&format!("write for run {} failed: {why}", self.id));
                }
            }
            R13 => {
                // HANG
                self.cancel.cancel();
                let why = match arg {
                    Arg::Why(w) => w,
                    _ => String::new(),
                };
                self.publish(inner, move |id, seq| err_frame(id, seq, "provider_timeout", &why));
                inner.finish_at = Some(Instant::now());
            }
            R14 => self.stop_orphan(inner), // SWEEP: the registry forgets a run that reached NoRun
            R15 => {
                // KEEPALIVE
                let (phase, idle) = self.vitals();
                self.notify_locked(inner, &Status { kind: "status", id: self.id.clone(), stage: "alive", phase: Some(phase.to_string()), idle_ms: Some(idle.as_millis() as i64) });
            }
            E1 | E2 | E3 => {
                let Arg::Reply { session, id } = arg else { unreachable!("an error action carries the reply target") };
                let (code, msg) = match action {
                    E1 => ("unknown_ask", "this answer is no longer kept"),
                    E2 => ("bad_request", "duplicate id"),
                    _ => ("busy", "another question is in progress"),
                };
                session.send(&ErrorFrame::new(&id, code, msg));
            }
        }
    }

    fn complete(self: &Arc<Self>, inner: &mut Inner, d: DoneInfo) {
        if let Some(remember) = d.remember {
            remember();
        }
        let (text, missing, truncated, pages, elapsed_ms) = (d.text, d.missing, d.truncated, d.pages, d.elapsed_ms);
        self.publish(inner, move |id, seq| Done { kind: "done", id: id.to_string(), seq, text, missing, truncated, pages, elapsed_ms });
        inner.finish_at = Some(Instant::now());
    }

    fn stop_orphan(&self, inner: &mut Inner) {
        if let Some(h) = inner.orphan.take() {
            h.abort();
        }
    }

    /// Numbers a frame, keeps it and sends it to the plugin if one is attached.
    fn publish<T: Serialize>(self: &Arc<Self>, inner: &mut Inner, frame: impl FnOnce(&str, i64) -> T) {
        let Ok(b) = serde_json::to_vec(&frame(&self.id, inner.frames.len() as i64)) else { return };
        inner.frames.push(b.clone());
        if let Some(s) = inner.to.clone() {
            self.send_locked(inner, &s, &b);
        }
    }

    /// Writes one frame; a failed write is the SEND_FAIL event (the frame stays kept).
    fn send_locked(self: &Arc<Self>, inner: &mut Inner, s: &Arc<Session>, b: &[u8]) {
        if let Err(e) = s.send_raw(b) {
            self.dispatch(inner, RunEvent::SendFails, Arg::SendErr(e), false);
        }
    }

    /// Sends a transient frame (progress). It is not kept: a plugin that reconnects only needs the answer.
    fn notify_locked<T: Serialize>(self: &Arc<Self>, inner: &mut Inner, v: &T) {
        let Some(s) = inner.to.clone() else { return };
        let Ok(b) = serde_json::to_vec(v) else { return };
        self.send_locked(inner, &s, &b);
    }

    /// Sends a progress frame while the answer is running.
    pub fn notify<T: Serialize>(self: &Arc<Self>, v: &T) {
        let mut inner = self.inner.lock().unwrap();
        if inner.state == RunState::AskingLive {
            self.notify_locked(&mut inner, v);
        }
    }

    /// Records a sign of life; a non-empty phase moves the answer forward to it (never back).
    pub fn sign(&self, phase: &'static str) {
        let mut v = self.vitals.lock().unwrap();
        v.last = Instant::now();
        if !phase.is_empty() && phase_rank(phase) > phase_rank(v.phase) {
            v.phase = phase;
        }
    }

    /// Where the AI is and how long since it last showed a sign of life.
    pub fn vitals(&self) -> (&'static str, Duration) {
        let v = self.vitals.lock().unwrap();
        (v.phase, v.last.elapsed())
    }

    #[cfg(test)]
    pub(super) fn dispatch_for_test(self: &Arc<Self>, ev: RunEvent, arg: Arg, spent: bool) {
        let mut inner = self.inner.lock().unwrap();
        self.dispatch(&mut inner, ev, arg, spent);
    }

    #[cfg(test)]
    pub(super) fn set_state_for_test(&self, s: RunState) {
        self.inner.lock().unwrap().state = s;
    }

    #[cfg(test)]
    pub(super) fn frames_for_test(&self) -> Vec<Vec<u8>> {
        self.inner.lock().unwrap().frames.clone()
    }
}

fn spawn(f: impl std::future::Future<Output = ()> + Send + 'static) -> Option<JoinHandle<()>> {
    tokio::runtime::Handle::try_current().ok().map(|h| h.spawn(f))
}

/// The bridge's table of answers in flight and recently finished.
pub struct Runs {
    m: Mutex<HashMap<String, Arc<Run>>>,
    /// How long a finished answer can still be fetched.
    pub retention: Duration,
    /// How many answers are kept.
    pub max: usize,
}

impl Default for Runs {
    fn default() -> Self {
        Runs { m: Mutex::new(HashMap::new()), retention: Duration::from_secs(600), max: 16 }
    }
}

impl Runs {
    pub fn get(&self, id: &str) -> Option<Arc<Run>> {
        self.m.lock().unwrap().get(id).cloned()
    }

    /// Registers a new run (the table's NoRun × Ask cell says where it begins) after sweeping old
    /// finished ones. For an id that already has a run it returns that run, and nothing is created.
    pub fn start(&self, id: &str, cancel: CancellationToken, log: LogFn, grace: Duration) -> Result<Arc<Run>, Arc<Run>> {
        let mut m = self.m.lock().unwrap();
        if let Some(r) = m.get(id) {
            return Err(r.clone());
        }
        self.sweep_locked(&mut m);
        let Cell::Do { next, .. } = cell(RunState::NoRun, RunEvent::Ask) else { unreachable!("NoRun × Ask creates the run (a test pins the table)") };
        let r = Run::new(id, cancel, next, log, grace);
        m.insert(id.to_string(), r.clone());
        Ok(r)
    }

    /// Fires SWEEP at finished runs that are past their retention (or beyond the cap); the ones the
    /// table sends to NoRun are forgotten.
    fn sweep_locked(&self, m: &mut HashMap<String, Arc<Run>>) {
        let ids: Vec<String> = m.keys().cloned().collect();
        for id in ids {
            let r = m[&id].clone();
            let forgotten = {
                let mut inner = r.inner.lock().unwrap();
                let finished = matches!(inner.state, RunState::DoneLive | RunState::DoneAway);
                let old = finished && inner.finish_at.is_some_and(|t| t.elapsed() > self.retention);
                if old || (m.len() >= self.max && finished) {
                    r.dispatch(&mut inner, RunEvent::Swept, Arg::None, false);
                }
                inner.state == RunState::NoRun
            };
            if forgotten {
                m.remove(&id);
            }
        }
    }
}

/// The row of the table for an id with no run (state NoRun). The registry applies it.
pub fn rule_n(ev: RunEvent) -> Cell {
    cell(RunState::NoRun, ev)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::run_table::{cell, Cell};
    use crate::server::transport::Wire;

    fn deps() -> Arc<crate::server::Deps> {
        use crate::provider::{self, AskFuture};
        Arc::new(crate::server::Deps {
            name: "t".into(),
            version: "0".into(),
            token: "secret".into(),
            font: None,
            provider: Arc::new(provider::func("t", |_, _, _, _| -> AskFuture<'static> { Box::pin(async { Ok(String::new()) }) })),
            report: Arc::new(|_, _| {}),
            log: Arc::new(|_| {}),
            settings: Arc::new(Default::default),
            set_settings: Arc::new(|_| Ok(())),
            history: Arc::new(crate::history::Store::none()),
            history_chars: 0,
            runs: Arc::new(Runs::default()),
            first_output: Duration::ZERO,
            gap: Duration::ZERO,
            total: Duration::ZERO,
            keepalive_every: Duration::ZERO,
            orphan_grace: Duration::ZERO,
            devices: None,
            sessions: Arc::new(Default::default()),
            pair_request_wait: Duration::ZERO,
            pairing_max: Duration::ZERO,
        })
    }

    fn logged() -> (LogFn, Arc<Mutex<Vec<String>>>) {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let l2 = lines.clone();
        (Arc::new(move |s| l2.lock().unwrap().push(s.to_string())), lines)
    }

    /// The argument the caller of an event hands to the table.
    fn arg_for(st: RunState, ev: RunEvent, s: &Arc<Session>) -> Arg {
        let done = || DoneInfo { text: "t".into(), missing: vec![], truncated: false, pages: 1, elapsed_ms: 1, remember: None };
        match ev {
            RunEvent::AskRepeats | RunEvent::AskWhileBusy => Arg::Reply { session: s.clone(), id: "x".into() },
            RunEvent::DeviceLeaves => Arg::Left(s.clone()),
            // with no run there is nobody to replay: the table answers "no longer kept" (E1)
            RunEvent::DeviceResumes if st == RunState::NoRun => Arg::Reply { session: s.clone(), id: "x".into() },
            RunEvent::DeviceResumes => Arg::Resume { session: s.clone(), next: 0 },
            RunEvent::TextArrives => Arg::Lines(vec![]),
            RunEvent::AiFinishes | RunEvent::PagesRunOut => Arg::Done(done()),
            RunEvent::AiFails => Arg::Fail { code: "provider_failed".into(), message: "x".into() },
            RunEvent::AiGoesSilent => Arg::Why("silent".into()),
            RunEvent::SendFails => Arg::SendErr("closed".into()),
            _ => Arg::None,
        }
    }

    #[tokio::test]
    async fn every_cell_does_what_the_table_says() {
        for st in RunState::ALL {
            for ev in RunEvent::ALL {
                for spent in [false, true] {
                    let (log, lines) = logged();
                    let (session, _rx) = Session::detached_for_test(deps());
                    let run = Run::new("x", CancellationToken::new(), *st, log, Duration::from_secs(3600));
                    if matches!(st, RunState::AskingLive | RunState::DoneLive) {
                        run.attach_first(&session);
                    }
                    let at = format!("{} × {}", st.name(), ev.name());
                    run.dispatch_for_test(*ev, arg_for(*st, *ev, &session), spent);
                    match cell(*st, *ev) {
                        Cell::Never(_) => {
                            assert_eq!(run.state(), *st, "{at}: a never cell stays");
                            assert!(lines.lock().unwrap().first().is_some_and(|l| l.starts_with("run.never")), "{at}: a never cell says so: {:?}", lines.lock().unwrap());
                        }
                        Cell::Ignore(_) | Cell::IgnoreDo(_) => assert_eq!(run.state(), *st, "{at}: an ignore cell stays"),
                        Cell::Do { next, .. } => assert_eq!(run.state(), next, "{at}"),
                        Cell::Guard { more, spent: sp } => assert_eq!(run.state(), if spent { sp.next } else { more.next }, "{at} (spent={spent})"),
                    }
                }
            }
        }
    }

    fn render(state: RunState, c: Cell) -> String {
        match c {
            Cell::Never(note) => format!("—{note}"),
            Cell::Ignore(note) => format!("×{note}"),
            Cell::IgnoreDo(a) => format!("×{}", a.name()),
            Cell::Do { action, next } if next == state => action.name().to_string(),
            Cell::Do { action, next } => format!("{}→{}", action.name(), next.name()),
            Cell::Guard { more, spent } => format!("{}→{} [more] / {}→{} [spent]", more.action.name(), more.next.name(), spent.action.name(), spent.next.name()),
        }
    }

    /// The document is the source of the table; the generated code must say the same in every cell.
    /// If this fails: change the document, then run `npm run spec` in plugin/.
    #[test]
    fn the_table_in_the_document_is_the_table_in_the_code() {
        let raw = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/spec/bridge-run.md")).expect("docs/spec/bridge-run.md");
        let (mut header, mut seen): (Option<Vec<RunEvent>>, usize) = (None, 0);
        for line in raw.lines() {
            if !line.starts_with('|') {
                continue;
            }
            let cells: Vec<String> = line.trim_matches(|c| c == '|' || c == ' ').split('|').map(|c| c.trim().to_string()).collect();
            if cells[0].starts_with("状態＼イベント") {
                header = Some(cells[1..].iter().map(|e| *RunEvent::ALL.iter().find(|x| x.name() == e).unwrap_or_else(|| panic!("event {e} is in the document but not in the code"))).collect());
                continue;
            }
            let Some(header) = &header else { continue };
            if !cells[0].starts_with("**") {
                continue;
            }
            let sname = cells[0].trim_matches('*');
            let state = *RunState::ALL.iter().find(|x| x.name() == sname).unwrap_or_else(|| panic!("state {sname} is in the document but not in the code"));
            for (i, e) in header.iter().enumerate() {
                let got = render(state, cell(state, *e));
                assert_eq!(got, cells[i + 1], "{} × {}: the document says {:?}, the code says {:?}", state.name(), e.name(), cells[i + 1], got);
                seen += 1;
            }
        }
        assert_eq!(seen, RunState::ALL.len() * RunEvent::ALL.len(), "the document has {seen} cells");
    }

    #[test]
    fn the_shape_of_the_table() {
        let next = |st: RunState| -> Vec<RunState> {
            RunEvent::ALL
                .iter()
                .flat_map(|ev| match cell(st, *ev) {
                    Cell::Do { next, .. } => vec![next],
                    Cell::Guard { more, spent } => vec![more.next, spent.next],
                    _ => vec![],
                })
                .collect()
        };
        // a run is created only from NoRun, and begins working
        assert!(matches!(cell(RunState::NoRun, RunEvent::Ask), Cell::Do { next: RunState::AskingLive, .. }), "NoRun × Ask must create a working run");
        // a finished run never goes back to work
        for st in [RunState::DoneLive, RunState::DoneAway] {
            for n in next(st) {
                assert!(n != RunState::AskingLive && n != RunState::AskingAway, "{} must not go back to work ({})", st.name(), n.name());
            }
        }
        // everything that is working can end
        for st in [RunState::AskingLive, RunState::AskingAway] {
            let mut seen = std::collections::HashSet::from([st]);
            let mut todo = vec![st];
            while let Some(s) = todo.pop() {
                for n in next(s) {
                    if seen.insert(n) {
                        todo.push(n);
                    }
                }
            }
            assert!(seen.contains(&RunState::DoneLive) && seen.contains(&RunState::DoneAway), "{} must be able to finish", st.name());
        }
        // the only way into NoRun from a run is the sweep
        for st in RunState::ALL {
            for ev in RunEvent::ALL {
                let targets = match cell(*st, *ev) {
                    Cell::Do { next, .. } => vec![next],
                    Cell::Guard { more, spent } => vec![more.next, spent.next],
                    _ => vec![],
                };
                for n in targets {
                    assert!(!(n == RunState::NoRun && *st != RunState::NoRun && *ev != RunEvent::Swept), "{} × {} leads to NoRun: only the sweep forgets a run", st.name(), ev.name());
                }
            }
        }
        // both ways of ending an answer in the middle are covered in every working state
        for st in [RunState::AskingLive, RunState::AskingAway] {
            for ev in [RunEvent::Cancel, RunEvent::AiFails, RunEvent::AiGoesSilent, RunEvent::AiFinishes, RunEvent::PagesRunOut] {
                assert!(matches!(cell(st, ev), Cell::Do { next: RunState::DoneLive | RunState::DoneAway, .. }), "{} × {} must finish the run", st.name(), ev.name());
            }
        }
    }

    #[tokio::test]
    async fn finished_answers_are_forgotten_after_their_retention() {
        let runs = Runs { retention: Duration::from_millis(30), ..Runs::default() };
        let quiet: LogFn = Arc::new(|_| {});
        let first = runs.start("old", CancellationToken::new(), quiet.clone(), Duration::ZERO).ok().unwrap();
        first.fire(RunEvent::AiFails, Arg::Fail { code: "provider_failed".into(), message: "x".into() }); // ends it
        assert!(first.is_finished(), "a failure ends the run");
        tokio::time::sleep(Duration::from_millis(60)).await;
        let _ = runs.start("new", CancellationToken::new(), quiet.clone(), Duration::ZERO);
        assert!(runs.get("old").is_none(), "a finished run past its retention is forgotten by the next start");
        let running = runs.start("running", CancellationToken::new(), quiet.clone(), Duration::ZERO).ok().unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
        let _ = runs.start("another", CancellationToken::new(), quiet, Duration::ZERO);
        assert!(runs.get("running").is_some() && !running.is_finished(), "a run that is still working is never forgotten");
    }

    #[tokio::test]
    async fn the_cap_forgets_finished_runs_only() {
        let runs = Runs { max: 3, ..Runs::default() };
        let quiet: LogFn = Arc::new(|_| {});
        for id in ["a", "b"] {
            let r = runs.start(id, CancellationToken::new(), quiet.clone(), Duration::ZERO).ok().unwrap();
            r.fire(RunEvent::AiFails, Arg::Fail { code: "provider_failed".into(), message: "x".into() });
        }
        let _running = runs.start("c", CancellationToken::new(), quiet.clone(), Duration::ZERO).ok().unwrap();
        let _ = runs.start("d", CancellationToken::new(), quiet, Duration::ZERO);
        assert!(runs.get("c").is_some(), "a working run is never swept, whatever the cap");
        assert!(runs.get("a").is_none() || runs.get("b").is_none(), "over the cap, finished runs are forgotten");
    }

    #[tokio::test]
    async fn a_start_for_a_known_id_returns_the_existing_run() {
        let runs = Runs::default();
        let quiet: LogFn = Arc::new(|_| {});
        let first = runs.start("same", CancellationToken::new(), quiet.clone(), Duration::ZERO).ok().unwrap();
        match runs.start("same", CancellationToken::new(), quiet, Duration::ZERO) {
            Err(existing) => assert!(Arc::ptr_eq(&existing, &first)),
            Ok(_) => panic!("a second run was created for the same id"),
        }
    }

    #[tokio::test]
    async fn a_keep_alive_is_not_sent_to_a_nobody() {
        // AskingAway × KeepaliveTick is an ignore cell: a run with nobody attached sends nothing
        // and stays where it is.
        let (session, mut rx) = Session::detached_for_test(deps());
        let run = Run::new("k", CancellationToken::new(), RunState::AskingLive, Arc::new(|_| {}), Duration::from_secs(3600));
        run.disconnect(&session); // a connection that is not the one receiving: not news
        assert_eq!(run.state(), RunState::AskingLive, "a disconnect from nobody changes nothing");
        run.set_state_for_test(RunState::AskingAway);
        run.fire(RunEvent::KeepaliveTick, Arg::None);
        assert_eq!(run.state(), RunState::AskingAway);
        assert!(rx.try_recv().is_err(), "nothing was sent");
    }

    #[tokio::test]
    async fn a_failed_write_is_logged_once_and_the_frames_are_kept() {
        let (log, lines) = logged();
        let (session, rx) = Session::detached_for_test(deps());
        drop(rx); // the writer is gone: every send fails
        let run = Run::new("w", CancellationToken::new(), RunState::AskingLive, log, Duration::from_secs(3600));
        run.attach_first(&session);
        for _ in 0..3 {
            run.fire(RunEvent::TextArrives, Arg::Lines(vec![InkLine { index: 0, page: 0, text: "あ".into(), strokes: vec![], cont: false }]));
        }
        assert_eq!(run.frame_count(), 3, "the frames are kept for a reconnect");
        assert_eq!(lines.lock().unwrap().iter().filter(|l| l.contains("write for run w failed")).count(), 1, "only the first failure is logged");
        assert_eq!(run.state(), RunState::AskingLive, "a failed write does not end the run");
    }

    #[tokio::test]
    async fn frames_are_numbered_from_zero_without_gaps() {
        let (session, mut rx) = Session::detached_for_test(deps());
        let run = Run::new("n", CancellationToken::new(), RunState::AskingLive, Arc::new(|_| {}), Duration::from_secs(3600));
        run.attach_first(&session);
        let line = |t: &str| InkLine { index: 0, page: 0, text: t.into(), strokes: vec![], cont: false };
        run.fire(RunEvent::TextArrives, Arg::Lines(vec![line("a"), line("b")]));
        run.fire(RunEvent::AiFinishes, Arg::Done(DoneInfo { text: "ab".into(), missing: vec![], truncated: false, pages: 1, elapsed_ms: 5, remember: None }));
        let mut seqs = vec![];
        while let Ok(Wire::Text(t)) = rx.try_recv() {
            seqs.push(serde_json::from_str::<serde_json::Value>(&t).unwrap()["seq"].as_i64().unwrap());
        }
        assert_eq!(seqs, vec![0, 1, 2]);
        assert_eq!(run.frames_for_test().len(), 3);
    }
}

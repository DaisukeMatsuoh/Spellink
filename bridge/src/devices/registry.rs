//! The ledger with its state machine (Go: internal/devices/registry.go). Safe for concurrent use.

use super::table::{self, Cell, DevAction, DevEvent, DevState};
use super::*;
use std::collections::HashMap;
use std::sync::{Mutex, Weak};
use std::time::Duration;

// Why a request for pairing is refused (the `reason` of a pairDenied frame; pairing.md, C).
pub const DENY_CODE_IN_USE: &str = "code_in_use";
pub const DENY_PAIR_BUSY: &str = "pair_busy";
pub const DENY_GRANT_PENDING: &str = "grant_pending";
pub const DENY_WINDOW_CLOSED: &str = "window_closed";
pub const DENY_EXPIRED: &str = "expired";
pub const DENY_SUPERSEDED: &str = "superseded";
pub const DENY_UNKNOWN_DEVICE: &str = "unknown_device";

/// A device's connection as the ledger sees it: it can be told it is granted K and it can be refused.
pub trait Conn: Send + Sync {
    /// Sends K (and where to reach the bridge) to the device. Called outside the ledger's lock.
    fn grant(&self, key: &str, endpoints: &[String]) -> Result<(), String>;
    /// Refuses the request with one of the Deny reasons and closes the connection.
    fn deny(&self, reason: &str);
}

fn same_conn(a: &Arc<dyn Conn>, b: &Arc<dyn Conn>) -> bool {
    std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b))
}

/// Connects the ledger to the rest of the bridge. Every one may be absent. They are called
/// outside the ledger's lock (Endpoints is called while an action runs, to build the grant).
#[derive(Clone, Default)]
pub struct Hooks {
    /// The URLs a device should use with secret k (it is sent in pairGrant).
    pub endpoints: Option<Arc<dyn Fn(&str, &str) -> Vec<String> + Send + Sync>>,
    /// Called after the set of valid secrets changed (rooms on the relay must follow).
    pub changed: Option<Arc<dyn Fn() + Send + Sync>>,
    /// Ends every live session of the device (it was revoked, reissued, or its K replaced).
    pub close_all: Option<Arc<dyn Fn(&str) + Send + Sync>>,
}

/// The ledger's limits. Zero values take the defaults of ADR 0016.
#[derive(Clone)]
pub struct Config {
    /// How long requests wait for the owner (default 90 s).
    pub window: Duration,
    /// How long B stays valid (default 24 h).
    pub ttl: Duration,
    /// How long to wait for the device to confirm it has K (default 30 s).
    pub ack_timeout: Duration,
    /// Waiting requests per device (default 4).
    pub max_pending: usize,
    /// Wrong codes per window (default 5).
    pub max_attempts: usize,
    pub now: Option<Arc<dyn Fn() -> Timestamp + Send + Sync>>,
    pub log: Option<Arc<dyn Fn(&str) + Send + Sync>>,
}

impl Default for Config {
    fn default() -> Self {
        Config { window: Duration::ZERO, ttl: Duration::ZERO, ack_timeout: Duration::ZERO, max_pending: 0, max_attempts: 0, now: None, log: None }
    }
}

impl Config {
    pub fn with_defaults(mut self) -> Config {
        if self.window.is_zero() {
            self.window = Duration::from_secs(90);
        }
        if self.ttl.is_zero() {
            self.ttl = Duration::from_secs(24 * 3600);
        }
        if self.ack_timeout.is_zero() {
            self.ack_timeout = Duration::from_secs(30);
        }
        if self.max_pending == 0 {
            self.max_pending = 4;
        }
        if self.max_attempts == 0 {
            self.max_attempts = 5;
        }
        if self.now.is_none() {
            self.now = Some(Arc::new(Timestamp::now));
        }
        if self.log.is_none() {
            self.log = Some(Arc::new(|_| {}));
        }
        self
    }
}

/// How often a device's last-connection time is written to the ledger.
pub const LAST_SEEN_EVERY: Duration = Duration::from_secs(60);

#[derive(Clone)]
struct Pending {
    code: String,
    conn: Arc<dyn Conn>,
}

/// A record plus what only lives in memory.
#[derive(Clone)]
struct Rec {
    record: Record,
    state: DevState,
    /// The state before the move that is running (an action sees the new state in `state`).
    prev: DevState,
    pending: Vec<Pending>,
    window_end: Timestamp,
    failures: usize,
    ack_end: Timestamp,
}

impl Rec {
    fn new(record: Record, state: DevState) -> Rec {
        Rec { record, state, prev: state, pending: vec![], window_end: Timestamp::ZERO, failures: 0, ack_end: Timestamp::ZERO }
    }
    fn index_of_code(&self, code: &str) -> Option<usize> {
        self.pending.iter().position(|p| p.code == code)
    }
}

/// What the caller of an event hands to the table.
enum Arg {
    None,
    Name(String),
    Request { code: String, conn: Arc<dyn Conn> },
    Conn(Arc<dyn Conn>),
    Chosen(Pending),
    Repack,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// The cell's action ran.
    Moved,
    /// The table says: happens, nothing to do.
    Ignored,
    /// The table says: cannot happen here (recorded).
    Never,
}

type Post = Box<dyn FnOnce() + Send>;

/// What an action did to the set of records.
#[derive(PartialEq, Eq)]
enum Member {
    Keep,
    Add,
    Remove,
}

struct Eff {
    posts: Vec<Post>,
    member: Member,
}

impl Eff {
    fn keep(posts: Vec<Post>) -> Result<Eff, Error> {
        Ok(Eff { posts, member: Member::Keep })
    }
}

/// What a failed action restores (ADR 0016, P7 in pairing.md).
struct Snapshot {
    rec: Rec,
    listed: bool,
    list_index: usize,
}

/// What the owner is told about one record (never a secret).
#[derive(Debug, Clone, PartialEq)]
pub struct Info {
    pub id: String,
    pub name: String,
    pub generation: i64,
    /// issued, waiting, granting, active, expired, revoked
    pub status: String,
    /// Requests waiting for the owner's code.
    pub waiting: usize,
    pub issued_at: Timestamp,
    /// When B stops being valid (issued, waiting and granting only).
    pub expires: Timestamp,
    pub last_seen: Timestamp,
}

/// What the .snplg for a device carries (REPACK).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PackageData {
    pub id: String,
    pub name: String,
    pub generation: i64,
    /// Empty once the device is paired (it keeps K itself).
    pub b: String,
    pub window: Duration,
}

/// One valid secret and the device it belongs to (the relay needs one room for each).
#[derive(Debug, Clone, PartialEq)]
pub struct Secret {
    pub id: String,
    pub secret: String,
    pub is_k: bool,
}

pub(super) struct Inner {
    cfg: Config,
    store: Box<dyn Store>,
    hooks: Hooks,
    rnd: Rand,
    recs: HashMap<String, Rec>,
    order: Vec<String>,
    saved: Option<Vec<u8>>,
    posts: Vec<Post>,
    changed: bool,
    repack: PackageData,
    me: Weak<Shared>,
}

struct Shared {
    inner: Mutex<Inner>,
}

/// The ledger. Cloning shares it.
#[derive(Clone)]
pub struct Registry(Arc<Shared>);

fn keeps_on_error(a: DevAction) -> bool {
    a == DevAction::P13
}

impl Inner {
    fn log(&self, s: &str) {
        (self.cfg.log.as_ref().expect("defaults"))(s);
    }
    fn now(&self) -> Timestamp {
        (self.cfg.now.as_ref().expect("defaults"))()
    }
    /// When B stops being valid.
    fn deadline(&self, rc: &Rec) -> Timestamp {
        rc.record.issued_at.add(self.cfg.ttl)
    }

    fn snap(&self, rc: &Rec) -> Snapshot {
        let idx = self.order.iter().position(|id| *id == rc.record.id);
        Snapshot { rec: rc.clone(), listed: idx.is_some(), list_index: idx.unwrap_or(0) }
    }

    /// Puts a record back as it was: where it was listed, or not listed at all.
    fn restore(&mut self, current_id: &str, s: Snapshot) {
        let id = s.rec.record.id.clone();
        let at = self.order.iter().position(|x| x == current_id);
        match (s.listed, at) {
            (true, None) => self.order.insert(s.list_index.min(self.order.len()), id.clone()),
            (false, Some(i)) => {
                self.order.remove(i);
                self.recs.remove(current_id);
            }
            _ => {}
        }
        if s.listed {
            self.recs.insert(id, s.rec);
        }
    }

    /// Writes the ledger when it differs from what is stored. Waiting requests are not stored: a
    /// record in Requested is stored as issued.
    fn save(&mut self) -> Result<(), String> {
        let recs: Vec<Record> = self
            .order
            .iter()
            .filter_map(|id| self.recs.get(id))
            .map(|rc| {
                let mut r = rc.record.clone();
                r.state = state_code(rc.state).to_string();
                r
            })
            .collect();
        let data = encode(&recs)?;
        if self.saved.as_deref() == Some(data.as_slice()) {
            return Ok(());
        }
        self.store.save(&data)?;
        self.saved = Some(data);
        Ok(())
    }

    /// The one place where a record's state changes, for a record that is not in the map (the
    /// caller took it out). `spent` picks the branch of a guarded cell (the caller decides it
    /// before the action runs; pairing.md, "条件つきのセル").
    fn fire_rec(&mut self, mut rc: Rec, ev: DevEvent, arg: Arg, spent: bool) -> (Outcome, Result<(), Error>) {
        let id_now = rc.record.id.clone();
        let reinsert = |me: &mut Inner, rc: Rec| {
            me.recs.insert(rc.record.id.clone(), rc);
        };
        let mv = match table::cell(rc.state, ev) {
            Cell::Never(note) => {
                self.log(&format!("devices.never: state {} event {} (note {note}) device {}", rc.state.name(), ev.name(), rc.record.id));
                if self.order.contains(&id_now) {
                    reinsert(self, rc);
                }
                return (Outcome::Never, Ok(()));
            }
            Cell::Ignore(_) | Cell::IgnoreDo(_) => {
                if self.order.contains(&id_now) {
                    reinsert(self, rc);
                }
                return (Outcome::Ignored, Ok(()));
            }
            Cell::Do { action, next } => table::DevMove { action, next },
            Cell::Guard { more, spent: s } => {
                if spent {
                    s
                } else {
                    more
                }
            }
        };
        let snap = self.snap(&rc);
        let from = rc.state;
        rc.prev = from;
        rc.state = mv.next;
        let eff = match self.run_action(mv.action, &mut rc, arg) {
            Ok(e) => e,
            Err(e) => {
                let current = rc.record.id.clone();
                self.restore(&current, snap);
                return (Outcome::Moved, Err(e));
            }
        };
        // the record is part of the ledger (or not) as the action says, then the ledger is saved
        let id = rc.record.id.clone();
        match eff.member {
            Member::Add => self.order.push(id.clone()),
            Member::Remove => self.order.retain(|x| *x != id),
            Member::Keep => {}
        }
        if eff.member != Member::Remove {
            reinsert(self, rc);
        }
        if let Err(e) = self.save() {
            if keeps_on_error(mv.action) {
                self.posts.extend(eff.posts);
                self.changed = true;
                return (Outcome::Moved, Err(Error::NotPersisted(e)));
            }
            self.restore(&id, snap);
            return (Outcome::Moved, Err(Error::Other(e)));
        }
        self.posts.extend(eff.posts);
        if from != mv.next {
            self.changed = true;
            self.log(&format!("devices.state: {id} {} -[{}]-> {} ({})", from.name(), ev.name(), mv.next.name(), mv.action.name()));
        }
        // An action that returns to Issued after B's time is up must not leave an expired B valid.
        if (mv.action == DevAction::P8 || mv.action == DevAction::P11) && self.recs.get(&id).is_some_and(|r| r.state == DevState::Issued && self.now() >= self.deadline(r)) {
            return self.fire_id(&id, DevEvent::TtlEnds, Arg::None, false);
        }
        (Outcome::Moved, Ok(()))
    }

    fn fire_id(&mut self, id: &str, ev: DevEvent, arg: Arg, spent: bool) -> (Outcome, Result<(), Error>) {
        match self.recs.remove(id) {
            Some(rc) => self.fire_rec(rc, ev, arg, spent),
            None => (Outcome::Never, Ok(())),
        }
    }

    fn deny(&self, rc: &mut Rec, reason: &'static str) -> Vec<Post> {
        let posts = rc.pending.drain(..).map(|p| -> Post { Box::new(move || p.conn.deny(reason)) }).collect();
        rc.window_end = Timestamp::ZERO;
        posts
    }

    fn close_all(&self, id: &str) -> Vec<Post> {
        match &self.hooks.close_all {
            None => vec![],
            Some(f) => {
                let (f, id) = (f.clone(), id.to_string());
                vec![Box::new(move || f(&id))]
            }
        }
    }

    fn run_action(&mut self, action: DevAction, rc: &mut Rec, arg: Arg) -> Result<Eff, Error> {
        use DevAction::*;
        match action {
            P1 => {
                // ISSUE
                let Arg::Name(name) = arg else { unreachable!("ISSUE carries the name") };
                if !valid_name(&name) {
                    return Err(Error::BadName);
                }
                if self.order.len() >= MAX_DEVICES {
                    return Err(Error::Full);
                }
                let mut tries = 0;
                let id = loop {
                    let id = new_id(&mut self.rnd).map_err(Error::Other)?;
                    if !self.recs.contains_key(&id) && !self.order.contains(&id) {
                        break id;
                    }
                    if tries >= 3 {
                        return Err(Error::Other("could not find a free device id".into()));
                    }
                    tries += 1;
                };
                let b = new_secret(&mut self.rnd).map_err(Error::Other)?;
                rc.record = Record { id, name, generation: 1, b, issued_at: self.now(), ..Record::default() };
                Ok(Eff { posts: vec![], member: Member::Add })
            }
            P2 => {
                // CONN_B: a request is queued
                let Arg::Request { code, conn } = arg else { unreachable!() };
                let mut posts: Vec<Post> = vec![];
                if rc.prev == DevState::Issued {
                    rc.window_end = self.now().add(self.cfg.window);
                    rc.failures = 0;
                }
                if let Some(i) = rc.index_of_code(&code) {
                    let other = rc.pending.remove(i).conn;
                    let mine = conn.clone();
                    posts.push(Box::new(move || other.deny(DENY_CODE_IN_USE)));
                    posts.push(Box::new(move || mine.deny(DENY_CODE_IN_USE)));
                } else if rc.pending.len() >= self.cfg.max_pending {
                    posts.push(Box::new(move || conn.deny(DENY_PAIR_BUSY)));
                } else {
                    rc.pending.push(Pending { code, conn });
                }
                if rc.pending.is_empty() {
                    rc.window_end = Timestamp::ZERO;
                }
                Eff::keep(posts)
            }
            P3 => {
                let Arg::Request { conn, .. } = arg else { unreachable!() };
                Eff::keep(vec![Box::new(move || conn.deny(DENY_GRANT_PENDING))])
            }
            P5 => {
                // DISCONNECT of a waiting request
                let Arg::Conn(c) = arg else { unreachable!() };
                if let Some(i) = rc.pending.iter().position(|p| same_conn(&p.conn, &c)) {
                    rc.pending.remove(i);
                }
                if rc.pending.is_empty() {
                    rc.window_end = Timestamp::ZERO;
                }
                Eff::keep(vec![])
            }
            P6 => {
                // CONFIRM_OK: grant K
                let Arg::Chosen(chosen) = arg else { unreachable!() };
                let k = new_secret(&mut self.rnd).map_err(Error::Other)?;
                let mut posts: Vec<Post> = vec![];
                for p in rc.pending.drain(..) {
                    if !same_conn(&p.conn, &chosen.conn) {
                        posts.push(Box::new(move || p.conn.deny(DENY_SUPERSEDED)));
                    }
                }
                rc.window_end = Timestamp::ZERO;
                rc.record.k = k.clone();
                rc.ack_end = self.now().add(self.cfg.ack_timeout);
                let (id, generation) = (rc.record.id.clone(), rc.record.generation);
                let eps = self.hooks.endpoints.as_ref().map(|f| f(&id, &k)).unwrap_or_default();
                let me = self.me.clone();
                let log = self.cfg.log.clone().expect("defaults");
                posts.push(Box::new(move || {
                    if let Err(e) = chosen.conn.grant(&k, &eps) {
                        log(&format!("devices: sending K to {id} failed: {e}"));
                        if let Some(shared) = me.upgrade() {
                            let _ = Registry(shared).locked(|inner| inner.timeout_grant(&id, generation));
                        }
                    }
                }));
                Eff::keep(posts)
            }
            P7 => {
                // CONFIRM_BAD
                rc.failures += 1;
                if rc.failures >= self.cfg.max_attempts {
                    return Eff::keep(self.deny(rc, DENY_WINDOW_CLOSED));
                }
                Eff::keep(vec![])
            }
            P8 => Eff::keep(self.deny(rc, DENY_WINDOW_CLOSED)),
            P9 => {
                rc.record.b.clear();
                rc.ack_end = Timestamp::ZERO;
                rc.record.last_seen = self.now();
                Eff::keep(vec![])
            }
            P10 => {
                let now = self.now();
                if now.since(rc.record.last_seen) >= LAST_SEEN_EVERY {
                    rc.record.last_seen = now;
                }
                Eff::keep(vec![])
            }
            P11 => {
                rc.record.k.clear();
                rc.ack_end = Timestamp::ZERO;
                Eff::keep(vec![])
            }
            P12 => {
                rc.record.b.clear();
                Eff::keep(self.deny(rc, DENY_EXPIRED))
            }
            P13 => {
                rc.record.b.clear();
                rc.record.k.clear();
                rc.record.revoked_at = self.now();
                rc.ack_end = Timestamp::ZERO;
                let mut posts = self.deny(rc, DENY_UNKNOWN_DEVICE);
                posts.extend(self.close_all(&rc.record.id));
                Eff::keep(posts)
            }
            P14 => {
                let b = new_secret(&mut self.rnd).map_err(Error::Other)?;
                let mut posts = self.deny(rc, DENY_SUPERSEDED);
                rc.record.generation += 1;
                rc.record.b = b;
                rc.record.k.clear();
                rc.record.issued_at = self.now();
                rc.record.revoked_at = Timestamp::ZERO;
                rc.failures = 0;
                rc.ack_end = Timestamp::ZERO;
                posts.extend(self.close_all(&rc.record.id));
                Eff::keep(posts)
            }
            P15 => {
                rc.pending.clear();
                rc.window_end = Timestamp::ZERO;
                rc.failures = 0;
                rc.ack_end = Timestamp::ZERO;
                if rc.state == DevState::Granted {
                    rc.ack_end = self.now().add(self.cfg.ack_timeout);
                }
                Eff::keep(vec![])
            }
            P17 => Ok(Eff { posts: vec![], member: Member::Remove }),
            P18 => {
                self.repack = PackageData { id: rc.record.id.clone(), name: rc.record.name.clone(), generation: rc.record.generation, b: rc.record.b.clone(), window: self.cfg.window };
                let _ = arg; // REPACK carries no argument: the result is read from `repack`
                Eff::keep(vec![])
            }
        }
    }

    /// The ACK_TIMEOUT event for a grant that did not get through (or was not confirmed in time).
    /// It does nothing if the device has moved on (an old timer).
    fn timeout_grant(&mut self, id: &str, generation: i64) -> Result<(), Error> {
        match self.recs.get(id) {
            Some(rc) if rc.record.generation == generation && rc.state == DevState::Granted => self.fire_id(id, DevEvent::AckTimesOut, Arg::None, false).1,
            _ => Ok(()),
        }
    }

    fn get(&self, id: &str) -> Result<&Rec, Error> {
        self.recs.get(id).ok_or(Error::UnknownDevice)
    }

    fn by_name(&self, name: &str) -> Option<String> {
        self.order.iter().find(|id| self.recs.get(*id).is_some_and(|r| r.record.name == name)).cloned()
    }

    fn secrets(&self, id: &str) -> Option<(String, String)> {
        let rc = self.recs.get(id)?;
        let (b, k) = match rc.state {
            DevState::Issued | DevState::Requested => (rc.record.b.clone(), String::new()),
            DevState::Granted => (rc.record.b.clone(), rc.record.k.clone()),
            DevState::Active => (String::new(), rc.record.k.clone()),
            _ => (String::new(), String::new()),
        };
        Some((b, k))
    }
}

impl Registry {
    /// Reads the ledger (the RESTART event of the table). A ledger file that cannot be read at all
    /// is an error; records that cannot be used are left out and listed in the second value.
    pub fn new(cfg: Config, store: Box<dyn Store>, hooks: Hooks, rnd: Rand) -> Result<(Registry, Vec<String>), String> {
        let cfg = cfg.with_defaults();
        let data = store.load().map_err(|e| format!("reading the device ledger: {e}"))?;
        let decoded = match &data {
            None => None,
            Some(d) => Some(decode(d)?),
        };
        let shared = Arc::new_cyclic(|me| Shared {
            inner: Mutex::new(Inner { cfg, store, hooks, rnd, recs: HashMap::new(), order: vec![], saved: None, posts: vec![], changed: false, repack: PackageData::default(), me: me.clone() }),
        });
        let reg = Registry(shared);
        let Some((recs, skipped)) = decoded else { return Ok((reg, vec![])) };
        {
            let mut inner = reg.0.inner.lock().unwrap();
            inner.saved = data;
            for rd in &recs {
                // check() has made sure the state is one of them
                let state = code_state(&rd.state).expect("a checked state");
                inner.order.push(rd.id.clone());
                // RESTART writes only when a record had to be normalised (a leftover B dropped). If
                // that write fails the bridge still starts: memory is right, the file is fixed by
                // the next write.
                let (_, r) = inner.fire_rec(Rec::new(rd.clone(), state), DevEvent::BridgeRestarts, Arg::None, false);
                if let Err(e) = r {
                    inner.log(&format!("devices: could not rewrite the ledger at start: {e}"));
                }
            }
            inner.log(&format!("devices: {} record(s) loaded, {} skipped", recs.len(), skipped.len()));
        }
        Ok((reg, skipped))
    }

    /// Runs `f` under the lock, then (outside it) the side effects the actions asked for.
    pub(super) fn locked<T>(&self, f: impl FnOnce(&mut Inner) -> T) -> T {
        let (out, posts, changed, hook) = {
            let mut inner = self.0.inner.lock().unwrap();
            let out = f(&mut inner);
            (out, std::mem::take(&mut inner.posts), std::mem::take(&mut inner.changed), inner.hooks.changed.clone())
        };
        for p in posts {
            p();
        }
        if changed {
            if let Some(h) = hook {
                h();
            }
        }
        out
    }

    /// ISSUE: a new device record with its first secret.
    pub fn add(&self, name: &str) -> Result<String, Error> {
        self.locked(|inner| {
            if let Some(existing) = inner.by_name(name) {
                let _ = inner.fire_id(&existing, DevEvent::OwnerAdds, Arg::Name(name.to_string()), false); // the table's ×d: nothing changes
                return Err(Error::NameTaken);
            }
            let rc = Rec::new(Record::default(), DevState::NoRecord);
            let (_, r) = inner.fire_rec(rc, DevEvent::OwnerAdds, Arg::Name(name.to_string()), false);
            r?;
            Ok(inner.order.last().cloned().expect("an added record is listed"))
        })
    }

    /// CONN_B: a device connected with B and sent a pairRequest.
    pub fn request(&self, id: &str, generation: i64, code: &str, c: Arc<dyn Conn>) -> Result<(), Error> {
        if !valid_id(id) || !valid_code(code) || generation < 1 {
            c.deny(DENY_UNKNOWN_DEVICE);
            return Err(Error::BadRequest);
        }
        self.locked(|inner| {
            let deny = |inner: &mut Inner, c: Arc<dyn Conn>| inner.posts.push(Box::new(move || c.deny(DENY_UNKNOWN_DEVICE)));
            let Some(rc) = inner.recs.get(id).filter(|r| r.record.generation == generation) else {
                deny(inner, c);
                return Err(Error::UnknownDevice);
            };
            let spent = matches!(rc.state, DevState::Issued | DevState::Requested) && rc.index_of_code(code).is_some() && rc.pending.len() == 1;
            let (out, r) = inner.fire_id(id, DevEvent::DeviceRequests, Arg::Request { code: code.to_string(), conn: c.clone() }, spent);
            if out != Outcome::Moved || r.is_err() {
                deny(inner, c);
            }
            r
        })
    }

    /// DISCONNECT: a connection that was waiting for the owner closed.
    pub fn disconnect(&self, c: &Arc<dyn Conn>) {
        let _ = self.locked(|inner| {
            let ids = inner.order.clone();
            for id in ids {
                let Some(rc) = inner.recs.get(&id) else { continue };
                if rc.pending.iter().any(|p| same_conn(&p.conn, c)) {
                    let spent = rc.pending.len() == 1;
                    return inner.fire_id(&id, DevEvent::RequestDrops, Arg::Conn(c.clone()), spent).1;
                }
            }
            Ok(())
        });
    }

    /// CONFIRM_OK or CONFIRM_BAD: the owner typed a code. Returns the name of the device that is
    /// now granted. A code that matches no waiting request, or more than one, is refused without
    /// saying which devices are waiting.
    pub fn confirm(&self, code: &str) -> Result<String, Error> {
        self.confirm_inner(None, code)
    }

    /// `confirm` for a code that came with the device's id (the QR code the device shows carries
    /// both): only that device's requests are looked at, and a wrong code counts against that
    /// device alone.
    pub fn confirm_for(&self, id: &str, code: &str) -> Result<String, Error> {
        if !valid_id(id) {
            return Err(Error::UnknownDevice);
        }
        self.confirm_inner(Some(id), code)
    }

    fn confirm_inner(&self, only_id: Option<&str>, code: &str) -> Result<String, Error> {
        if !valid_code(code) {
            return Err(Error::BadCode);
        }
        self.locked(|inner| {
            if let Some(id) = only_id {
                inner.get(id)?;
            }
            let mut waiting: Vec<String> = vec![];
            let mut hit: Option<(String, Pending)> = None;
            let mut hits = 0;
            for id in &inner.order {
                let Some(rc) = inner.recs.get(id) else { continue };
                if rc.state != DevState::Requested || only_id.is_some_and(|o| o != id) {
                    continue;
                }
                waiting.push(id.clone());
                for p in &rc.pending {
                    if p.code == code {
                        hit = Some((id.clone(), p.clone()));
                        hits += 1;
                    }
                }
            }
            if waiting.is_empty() {
                return Err(Error::NoPending);
            }
            if hits == 1 {
                let (id, p) = hit.expect("one hit");
                inner.fire_id(&id, DevEvent::OwnerConfirms, Arg::Chosen(p), false).1?;
                return Ok(inner.get(&id)?.record.name.clone());
            }
            for id in waiting {
                let spent = inner.recs.get(&id).is_some_and(|rc| rc.failures + 1 >= inner.cfg.max_attempts);
                inner.fire_id(&id, DevEvent::OwnerMistypes, Arg::None, spent).1?;
            }
            Err(Error::CodeRejected)
        })
    }

    /// ACK: the device says it has stored K.
    pub fn ack(&self, id: &str, generation: i64) -> Result<(), Error> {
        self.locked(|inner| {
            let rc = inner.get(id).map_err(|_| Error::UnknownDevice)?;
            if rc.record.generation != generation {
                return Err(Error::UnknownDevice);
            }
            inner.fire_id(id, DevEvent::DeviceAcks, Arg::None, false).1
        })
    }

    /// CONN_K: a session was authenticated with K. In Granted this also counts as the device's
    /// confirmation that it has K.
    pub fn connect_k(&self, id: &str) -> Result<(), Error> {
        self.locked(|inner| {
            inner.get(id)?;
            let (out, r) = inner.fire_id(id, DevEvent::KeyConnects, Arg::None, false);
            match r {
                Err(e) => Err(e),
                Ok(()) if out != Outcome::Moved => Err(Error::NotEligible),
                Ok(()) => Ok(()),
            }
        })
    }

    /// The secrets a device may connect with now: B while it can still pair (Issued, Requested,
    /// Granted), K once it has one (Granted, Active). `None` when it has none.
    pub fn secrets(&self, id: &str) -> Option<(String, String)> {
        self.locked(|inner| inner.secrets(id)).filter(|(b, k)| !b.is_empty() || !k.is_empty())
    }

    /// Every secret that may connect now.
    pub fn active_secrets(&self) -> Vec<Secret> {
        let ids = self.locked(|inner| inner.order.clone());
        let mut out = vec![];
        for id in ids {
            let (b, k) = self.locked(|inner| inner.secrets(&id)).unwrap_or_default();
            if !b.is_empty() {
                out.push(Secret { id: id.clone(), secret: b, is_k: false });
            }
            if !k.is_empty() {
                out.push(Secret { id: id.clone(), secret: k, is_k: true });
            }
        }
        out
    }

    /// REVOKE.
    pub fn revoke(&self, id: &str) -> Result<(), Error> {
        self.locked(|inner| {
            inner.get(id)?;
            let (out, r) = inner.fire_id(id, DevEvent::OwnerRevokes, Arg::None, false);
            if out == Outcome::Ignored {
                return Err(Error::AlreadyRevoked);
            }
            r
        })
    }

    /// REISSUE: a new first secret; the device must pair again.
    pub fn reissue(&self, id: &str) -> Result<(), Error> {
        self.locked(|inner| {
            inner.get(id)?;
            let (out, r) = inner.fire_id(id, DevEvent::OwnerReissues, Arg::None, false);
            if out == Outcome::Ignored {
                return Err(Error::Revoked);
            }
            r
        })
    }

    /// FORGET: delete a revoked or expired record.
    pub fn forget(&self, id: &str) -> Result<(), Error> {
        self.locked(|inner| {
            inner.get(id)?;
            let (out, r) = inner.fire_id(id, DevEvent::OwnerForgets, Arg::None, false);
            if out == Outcome::Ignored {
                return Err(Error::StillValid);
            }
            r
        })
    }

    /// REPACK: what a new .snplg for the device must carry. It changes nothing.
    pub fn repack(&self, id: &str) -> Result<PackageData, Error> {
        self.locked(|inner| {
            inner.get(id)?;
            let (out, r) = inner.fire_id(id, DevEvent::OwnerRepacks, Arg::Repack, false);
            r?;
            match (out, inner.recs.get(id).map(|r| r.state)) {
                (Outcome::Ignored, Some(DevState::Revoked)) => Err(Error::Revoked),
                (Outcome::Ignored, _) => Err(Error::Expired),
                _ => Ok(std::mem::take(&mut inner.repack)),
            }
        })
    }

    /// Fires the timers that are due (WINDOW, ACK_TIMEOUT, TTL). The caller runs it about once a second.
    pub fn tick(&self) {
        self.locked(|inner| {
            let now = inner.now();
            for id in inner.order.clone() {
                // a timer never adds or removes a record
                let Some(rc) = inner.recs.get(&id) else { continue };
                if rc.state == DevState::Requested && !rc.window_end.is_zero() && now >= rc.window_end {
                    let _ = inner.fire_id(&id, DevEvent::WindowEnds, Arg::None, false);
                }
                if let Some(rc) = inner.recs.get(&id) {
                    if rc.state == DevState::Granted && !rc.ack_end.is_zero() && now >= rc.ack_end {
                        let _ = inner.fire_id(&id, DevEvent::AckTimesOut, Arg::None, false);
                    }
                }
                if let Some(rc) = inner.recs.get(&id) {
                    if matches!(rc.state, DevState::Issued | DevState::Requested | DevState::Granted) && now >= inner.deadline(rc) {
                        let _ = inner.fire_id(&id, DevEvent::TtlEnds, Arg::None, false);
                    }
                }
            }
        })
    }

    /// Every record in the order they were added.
    pub fn list(&self) -> Vec<Info> {
        self.locked(|inner| {
            inner
                .order
                .iter()
                .filter_map(|id| inner.recs.get(id))
                .map(|rc| Info {
                    id: rc.record.id.clone(),
                    name: rc.record.name.clone(),
                    generation: rc.record.generation,
                    status: match rc.state {
                        DevState::Issued => "issued",
                        DevState::Requested => "waiting",
                        DevState::Granted => "granting",
                        DevState::Active => "active",
                        DevState::Expired => "expired",
                        DevState::Revoked => "revoked",
                        DevState::NoRecord => "",
                    }
                    .to_string(),
                    waiting: rc.pending.len(),
                    issued_at: rc.record.issued_at,
                    expires: if rc.record.b.is_empty() { Timestamp::ZERO } else { inner.deadline(rc) },
                    last_seen: rc.record.last_seen,
                })
                .collect()
        })
    }
}

#[cfg(test)]
mod tests;

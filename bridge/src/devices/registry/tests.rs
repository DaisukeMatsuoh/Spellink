//! The ledger's behaviour (Go: registry_test.go, ledger_test.go, table_test.go, doc_test.go).

use super::*;
use crate::devices::table::{DevAction, DevEvent, DevState};
use std::sync::Mutex;

// ---- the harness ----

#[derive(Clone)]
pub(crate) struct FakeClock(Arc<Mutex<Timestamp>>);
impl FakeClock {
    fn now(&self) -> Timestamp {
        *self.0.lock().unwrap()
    }
    fn advance(&self, d: Duration) {
        let mut t = self.0.lock().unwrap();
        *t = t.add(d);
    }
    fn set(&self, t: Timestamp) {
        *self.0.lock().unwrap() = t;
    }
}

#[derive(Default)]
pub(crate) struct MemInner {
    pub data: Option<Vec<u8>>,
    pub saves: usize,
    pub load_err: Option<String>,
    pub save_err: Option<String>,
}

/// A store in memory that can be made to fail.
#[derive(Clone, Default)]
pub(crate) struct MemStore(pub Arc<Mutex<MemInner>>);
impl Store for MemStore {
    fn load(&self) -> Result<Option<Vec<u8>>, String> {
        let m = self.0.lock().unwrap();
        match &m.load_err {
            Some(e) => Err(e.clone()),
            None => Ok(m.data.clone()),
        }
    }
    fn save(&self, b: &[u8]) -> Result<(), String> {
        let mut m = self.0.lock().unwrap();
        if let Some(e) = &m.save_err {
            return Err(e.clone());
        }
        m.saves += 1;
        m.data = Some(b.to_vec());
        Ok(())
    }
}
impl MemStore {
    fn data(&self) -> Option<Vec<u8>> {
        self.0.lock().unwrap().data.clone()
    }
    fn saves(&self) -> usize {
        self.0.lock().unwrap().saves
    }
    fn fail_saves(&self, e: Option<&str>) {
        self.0.lock().unwrap().save_err = e.map(String::from);
    }
}

/// Records what the ledger tells a device.
#[derive(Default)]
pub(crate) struct FakeConn {
    grants: Mutex<Vec<(String, Vec<String>)>>,
    denied: Mutex<Vec<String>>,
    grant_err: Option<String>,
    /// captures the ledger's bytes when K is sent: it must already be stored
    store: Option<MemStore>,
    at_grant: Mutex<Option<Vec<u8>>>,
}
impl Conn for FakeConn {
    fn grant(&self, key: &str, eps: &[String]) -> Result<(), String> {
        self.grants.lock().unwrap().push((key.to_string(), eps.to_vec()));
        if let Some(s) = &self.store {
            *self.at_grant.lock().unwrap() = s.data();
        }
        match &self.grant_err {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }
    fn deny(&self, reason: &str) {
        self.denied.lock().unwrap().push(reason.to_string());
    }
}
impl FakeConn {
    fn reasons(&self) -> String {
        self.denied.lock().unwrap().join(",")
    }
    fn grants(&self) -> Vec<(String, Vec<String>)> {
        self.grants.lock().unwrap().clone()
    }
}

/// Yields distinct bytes (n, n+1, …) so every secret and id differs, deterministically.
fn counter_rand(start: u8) -> Rand {
    let mut n = start;
    Box::new(move |p: &mut [u8]| {
        for b in p.iter_mut() {
            *b = n;
            n = n.wrapping_add(1);
        }
        Ok(())
    })
}

/// Serves the queued chunks first, then continues with a counter.
fn script_rand(queue: Arc<Mutex<Vec<Vec<u8>>>>) -> Rand {
    let mut rest = counter_rand(0);
    Box::new(move |p: &mut [u8]| {
        let next = {
            let mut q = queue.lock().unwrap();
            if q.is_empty() { None } else { Some(q.remove(0)) }
        };
        match next {
            Some(chunk) => {
                p.copy_from_slice(&chunk[..p.len()]);
                Ok(())
            }
            None => rest(p),
        }
    })
}

fn fail_rand() -> Rand {
    Box::new(|_: &mut [u8]| Err("no randomness".to_string()))
}

/// Serves n bytes and then fails.
fn after_n(mut n: usize) -> Rand {
    Box::new(move |p: &mut [u8]| {
        if n < p.len() {
            return Err("out of randomness".into());
        }
        n -= p.len();
        for (i, b) in p.iter_mut().enumerate() {
            *b = i as u8;
        }
        Ok(())
    })
}

#[derive(Default)]
struct HookLog {
    changed: usize,
    closed: Vec<String>,
    endpoints: Vec<(String, String)>,
}

pub(crate) struct Harness {
    reg: Registry,
    clock: FakeClock,
    store: MemStore,
    hooks: Arc<Mutex<HookLog>>,
    logs: Arc<Mutex<String>>,
}

fn hooks_for(h: &Arc<Mutex<HookLog>>) -> Hooks {
    let (a, b, c) = (h.clone(), h.clone(), h.clone());
    Hooks {
        endpoints: Some(Arc::new(move |id, k| {
            a.lock().unwrap().endpoints.push((id.to_string(), k.to_string()));
            vec![format!("wss://bridge.example/v1?d={id}")]
        })),
        changed: Some(Arc::new(move || b.lock().unwrap().changed += 1)),
        close_all: Some(Arc::new(move |id| c.lock().unwrap().closed.push(id.to_string()))),
    }
}

fn harness() -> Harness {
    harness_with(|_| {})
}

fn harness_with(mutate: impl FnOnce(&mut Config)) -> Harness {
    harness_rand(counter_rand(0), mutate)
}

fn harness_rand(rnd: Rand, mutate: impl FnOnce(&mut Config)) -> Harness {
    let clock = FakeClock(Arc::new(Mutex::new(Timestamp::ymd_hms(2026, 11, 20, 9, 0, 0))));
    let logs = Arc::new(Mutex::new(String::new()));
    let (c2, l2) = (clock.clone(), logs.clone());
    let mut cfg = Config { now: Some(Arc::new(move || c2.now())), log: Some(Arc::new(move |s| l2.lock().unwrap().push_str(&format!("{s}\n")))), ..Config::default() };
    mutate(&mut cfg);
    let store = MemStore::default();
    let hooks = Arc::new(Mutex::new(HookLog::default()));
    let (reg, skipped) = Registry::new(cfg, Box::new(store.clone()), hooks_for(&hooks), rnd).expect("new");
    assert!(skipped.is_empty(), "New skipped records: {skipped:?}");
    Harness { reg, clock, store, hooks, logs }
}

impl Harness {
    fn cfg(&self) -> Config {
        self.reg.locked(|i| i.cfg.clone())
    }
    fn set_rand(&self, r: Rand) {
        self.reg.locked(|i| i.rnd = r);
    }
    fn add(&self, name: &str) -> String {
        self.reg.add(name).unwrap_or_else(|e| panic!("add({name:?}): {e}"))
    }
    fn gen_of(&self, id: &str) -> i64 {
        self.reg.list().into_iter().find(|i| i.id == id).unwrap_or_else(|| panic!("no device {id}")).generation
    }
    fn request(&self, id: &str, code: &str) -> Arc<FakeConn> {
        let c = Arc::new(FakeConn { store: Some(self.store.clone()), ..FakeConn::default() });
        self.reg.request(id, self.gen_of(id), code, c.clone()).expect("request");
        c
    }
    fn status(&self, id: &str) -> String {
        self.reg.list().into_iter().find(|i| i.id == id).map(|i| i.status).unwrap_or_else(|| "gone".into())
    }
    fn must_status(&self, id: &str, want: &str) {
        assert_eq!(self.status(id), want, "status of {id}");
    }
    /// A new device taken through request → confirm: granted.
    fn granted(&self, name: &str, code: &str) -> (String, Arc<FakeConn>) {
        let id = self.add(name);
        let c = self.request(&id, code);
        assert_eq!(self.reg.confirm(code).as_deref(), Ok(name));
        (id, c)
    }
    fn active(&self, name: &str, code: &str) -> (String, Arc<FakeConn>) {
        let (id, c) = self.granted(name, code);
        self.reg.ack(&id, self.gen_of(&id)).expect("ack");
        (id, c)
    }
    /// What the store holds now.
    fn ledger(&self) -> Vec<Record> {
        let (recs, skipped) = decode(&self.store.data().expect("a stored ledger")).expect("stored ledger unusable");
        assert!(skipped.is_empty(), "{skipped:?}");
        recs
    }
    fn closed(&self) -> Vec<String> {
        self.hooks.lock().unwrap().closed.clone()
    }
    fn changed(&self) -> usize {
        self.hooks.lock().unwrap().changed
    }
    fn logs(&self) -> String {
        self.logs.lock().unwrap().clone()
    }
    fn stored_state(&self, name: &str) -> String {
        self.ledger().into_iter().find(|r| r.name == name).map(|r| r.state).unwrap_or_else(|| "not in the file".into())
    }
}

fn secret(n: u8) -> String {
    URL_SAFE_NO_PAD.encode([n; SECRET_BYTES])
}

fn good_record(state: &str) -> Record {
    let mut r = Record { id: "0123456789abcdef".into(), name: "Nomad".into(), generation: 1, state: state.into(), issued_at: Timestamp::ymd_hms(2026, 11, 20, 0, 0, 0), ..Record::default() };
    match state {
        STATE_ISSUED => r.b = secret(1),
        STATE_GRANTED => {
            r.b = secret(1);
            r.k = secret(2);
        }
        STATE_ACTIVE => r.k = secret(2),
        _ => {}
    }
    r
}

fn ledger_of(recs: &[Record]) -> Vec<u8> {
    encode(recs).expect("encode")
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap()
}

// ---- the ledger file ----

#[test]
fn validators() {
    for (n, want) in [("Nomad", true), ("ノート", true), (&"x".repeat(64), true), ("", false), (&"x".repeat(65), false), ("a\nb", false), ("a\0b", false)] {
        assert_eq!(valid_name(n), want, "valid_name({n:?})");
    }
    let secrets = [
        (secret(7), true),
        (String::new(), false),
        ("short".into(), false),
        (format!("{}=", secret(7)), false),
        ("A".repeat(44), false),
        ("!".repeat(43), false),
        // 43 characters whose last one leaves bits set that a 32-byte value cannot have: not canonical
        (format!("{}B", "A".repeat(42)), false),
    ];
    for (s, want) in secrets {
        assert_eq!(valid_secret(&s), want, "valid_secret({s:?})");
    }
    for (c, want) in [("12345678", true), ("00000000", true), ("1234567", false), ("123456789", false), ("1234567a", false), ("", false), ("１２３４５６７８", false)] {
        assert_eq!(valid_code(c), want, "valid_code({c:?})");
    }
    for (i, want) in [("0123456789abcdef", true), ("0123456789ABCDEF", false), ("0123456789abcde", false), ("0123456789abcdeg", false)] {
        assert_eq!(valid_id(i), want, "valid_id({i:?})");
    }
}

#[test]
fn decode_keeps_good_records_of_every_state() {
    let mk = |state: &str, id: &str, name: &str| Record { id: id.into(), name: name.into(), ..good_record(state) };
    let (recs, skipped) = decode(&ledger_of(&[
        good_record(STATE_ISSUED),
        mk(STATE_GRANTED, "1123456789abcdef", "g"),
        mk(STATE_ACTIVE, "2123456789abcdef", "a"),
        mk(STATE_EXPIRED, "3123456789abcdef", "x"),
        mk(STATE_REVOKED, "4123456789abcdef", "r"),
    ]))
    .unwrap();
    assert!(skipped.is_empty() && recs.len() == 5, "recs {} skipped {skipped:?}", recs.len());
}

#[test]
fn decode_leaves_out_unusable_records_and_says_why() {
    let mutated = |f: &dyn Fn(&mut Record)| {
        let mut r = good_record(STATE_ACTIVE);
        f(&mut r);
        r
    };
    let bad: Vec<(&str, Record)> = vec![
        ("bad id", mutated(&|r| r.id = "xyz".into())),
        ("bad name", mutated(&|r| r.name = String::new())),
        ("bad generation", mutated(&|r| r.generation = 0)),
        ("generation too large", mutated(&|r| r.generation = MAX_GEN + 1)),
        ("bad B", mutated(&|r| r.b = "nope".into())),
        ("bad K", mutated(&|r| r.k = "nope".into())),
        ("bad state", mutated(&|r| r.state = "requested".into())),
        (
            "issued without B",
            mutated(&|r| {
                r.state = STATE_ISSUED.into();
                r.k = String::new();
            }),
        ),
        (
            "issued with K",
            mutated(&|r| {
                r.state = STATE_ISSUED.into();
                r.b = secret(1);
            }),
        ),
        (
            "granting without K",
            mutated(&|r| {
                r.state = STATE_GRANTED.into();
                r.b = secret(1);
                r.k = String::new();
            }),
        ),
        ("active without K", mutated(&|r| r.k = String::new())),
        (
            "expired with a B",
            mutated(&|r| {
                r.state = STATE_EXPIRED.into();
                r.b = secret(1);
                r.k = String::new();
            }),
        ),
        ("revoked with a K", mutated(&|r| r.state = STATE_REVOKED.into())),
    ];
    for (name, rec) in bad {
        let (recs, skipped) = decode(&ledger_of(&[rec])).unwrap();
        assert!(recs.is_empty() && skipped.len() == 1, "{name}: recs {} skipped {skipped:?}", recs.len());
    }
}

#[test]
fn decode_drops_a_leftover_b_from_an_active_record() {
    let mut r = good_record(STATE_ACTIVE);
    r.b = secret(9);
    let (recs, skipped) = decode(&ledger_of(&[r])).unwrap();
    assert!(skipped.is_empty() && recs.len() == 1 && recs[0].b.is_empty() && recs[0].k == secret(2), "{recs:?} {skipped:?}");
}

#[test]
fn decode_refuses_duplicates_and_too_many() {
    let a = good_record(STATE_ISSUED);
    let same_id = Record { name: "other".into(), ..good_record(STATE_ISSUED) };
    let same_name = Record { id: "ffffffffffffffff".into(), ..good_record(STATE_ISSUED) };
    let (recs, skipped) = decode(&ledger_of(&[a, same_id, same_name])).unwrap();
    assert!(recs.len() == 1 && skipped.len() == 2, "recs {} skipped {skipped:?}", recs.len());
    let many: Vec<Record> = (0..MAX_DEVICES + 2).map(|i| Record { id: format!("{:016x}", i + 1), name: format!("d{i}"), ..good_record(STATE_ISSUED) }).collect();
    let (recs, skipped) = decode(&ledger_of(&many)).unwrap();
    assert!(recs.len() == MAX_DEVICES && skipped.len() == 2, "recs {} skipped {skipped:?}", recs.len());
}

#[test]
fn decode_refuses_a_file_that_is_not_a_ledger() {
    for data in [&b""[..], b"{{", b"[]", b"null", br#"{"version":2,"devices":[]}"#, br#"{"devices":[]}"#, br#"{"version":"1"}"#] {
        assert!(decode(data).is_err(), "{:?}", String::from_utf8_lossy(data));
    }
    assert!(decode(br#"{"version":1,"devices":[]} trailing"#).is_ok(), "like Go's decoder, the first value counts");
}

#[test]
fn a_ledger_written_by_the_go_bridge_reads_here() {
    // local-time offsets and a fraction, as Go's time.Time writes them
    let json = r#"{"version":1,"devices":[{"id":"0123456789abcdef","name":"Nomad","gen":2,"state":"active","k":"KKKK","issuedAt":"2026-11-20T18:00:00.123456789+09:00","lastSeen":"2026-11-21T08:15:00+09:00"}]}"#
        .replace("KKKK", &secret(2));
    let (recs, skipped) = decode(json.as_bytes()).unwrap();
    assert!(skipped.is_empty() && recs.len() == 1, "{skipped:?}");
    assert_eq!(recs[0].generation, 2);
    assert_eq!(recs[0].issued_at.to_rfc3339().unwrap(), "2026-11-20T09:00:00.123456789Z");
    assert!(recs[0].revoked_at.is_zero());
}

#[test]
fn new_reads_the_ledger_like_a_restart() {
    let h = harness();
    let (id, _) = h.active("A", "11111111");
    let granted = h.granted("G", "22222222").0;
    let issued = h.add("I");
    let revoked = h.add("R");
    h.reg.revoke(&revoked).unwrap();
    let (reg, skipped) = Registry::new(Config::default(), Box::new(h.store.clone()), Hooks::default(), counter_rand(100)).unwrap();
    assert!(skipped.is_empty());
    let status = |id: &str| reg.list().into_iter().find(|i| i.id == id).unwrap().status;
    assert_eq!(status(&id), "active");
    assert_eq!(status(&granted), "granting", "a grant that was not confirmed is still granting after a restart");
    assert_eq!(status(&issued), "issued");
    assert_eq!(status(&revoked), "revoked");
    assert_eq!(reg.list().iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["A", "G", "I", "R"], "the order is kept");
}

#[test]
fn new_reports_records_it_skips_and_refuses_an_unreadable_ledger() {
    let mut bad = good_record(STATE_ACTIVE);
    bad.k = String::new();
    let store = MemStore::default();
    store.0.lock().unwrap().data = Some(ledger_of(&[good_record(STATE_ISSUED), bad]));
    let (r, skipped) = Registry::new(Config::default(), Box::new(store), Hooks::default(), counter_rand(0)).unwrap();
    assert!(skipped.len() == 1 && r.list().len() == 1, "{skipped:?}");
    let unreadable = MemStore::default();
    unreadable.0.lock().unwrap().data = Some(b"{{".to_vec());
    assert!(Registry::new(Config::default(), Box::new(unreadable), Hooks::default(), counter_rand(0)).is_err(), "an unreadable ledger must stop the bridge");
    let failing = MemStore::default();
    failing.0.lock().unwrap().load_err = Some("io error".into());
    assert!(Registry::new(Config::default(), Box::new(failing), Hooks::default(), counter_rand(0)).is_err(), "a ledger that cannot be read must stop the bridge");
    let (r, skipped) = Registry::new(Config::default(), Box::new(MemStore::default()), Hooks::default(), counter_rand(0)).unwrap();
    assert!(skipped.is_empty() && r.list().is_empty(), "no ledger yet");
}

#[test]
fn a_leftover_b_is_dropped_and_the_next_write_saves_it() {
    let mut r = good_record(STATE_ACTIVE);
    r.b = secret(9);
    let store = MemStore::default();
    store.0.lock().unwrap().data = Some(ledger_of(std::slice::from_ref(&r)));
    let (reg, skipped) = Registry::new(Config::default(), Box::new(store.clone()), Hooks::default(), counter_rand(0)).unwrap();
    assert!(skipped.is_empty());
    assert_eq!(reg.secrets(&r.id).map(|(b, _)| b), Some(String::new()), "B must not be valid for an active device");
    reg.add("another").unwrap();
    let (recs, _) = decode(&store.data().unwrap()).unwrap();
    assert!(recs[0].b.is_empty(), "the leftover B was written back");
}

#[test]
fn starting_with_a_ledger_that_needs_rewriting_and_cannot_be_written_still_serves() {
    let mut r = good_record(STATE_ACTIVE);
    r.b = secret(9);
    let store = MemStore::default();
    {
        let mut m = store.0.lock().unwrap();
        m.data = Some(ledger_of(std::slice::from_ref(&r)));
        m.save_err = Some("read-only file system".into());
    }
    let logs = Arc::new(Mutex::new(String::new()));
    let l2 = logs.clone();
    let cfg = Config { log: Some(Arc::new(move |s| l2.lock().unwrap().push_str(s))), ..Config::default() };
    let (reg, _) = Registry::new(cfg, Box::new(store), Hooks::default(), counter_rand(0)).expect("new");
    assert_eq!(reg.secrets(&r.id).map(|(_, k)| k), Some(secret(2)), "an active device must still be served");
    assert!(lock(&logs).contains("could not rewrite"), "not logged: {:?}", lock(&logs));
}

fn file_store(dir: &std::path::Path) -> FileStore {
    FileStore::new(dir.join("devices.json"))
}

#[test]
fn file_store_writes_atomically_with_private_permissions() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().join("sub");
    let s = file_store(&dir);
    assert_eq!(s.load(), Ok(None), "Load of a missing file");
    s.save(b"one").unwrap();
    s.save(b"two").unwrap();
    assert_eq!(s.load().unwrap().as_deref(), Some(&b"two"[..]));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&s.path).unwrap().permissions().mode() & 0o777, 0o600);
    }
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "temporary files left behind");
}

#[test]
fn file_store_failures() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    // the directory cannot be created: its parent is a file
    std::fs::write(root.join("file"), b"").unwrap();
    assert!(FileStore::new(root.join("file/x/devices.json")).save(b"x").is_err(), "creating a directory under a file should fail");
    // the target is a directory: the rename fails, and the temporary file is cleaned up
    let dir = root.join("dir");
    std::fs::create_dir_all(dir.join("devices.json/keep")).unwrap();
    assert!(file_store(&dir).save(b"x").is_err(), "renaming over a directory should fail");
    let left: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().starts_with(".devices-")).collect();
    assert!(left.is_empty(), "temporary file left: {left:?}");
    // Load reports an error that is not "does not exist"
    assert!(FileStore::new(&dir).load().is_err(), "reading a directory should fail");
}

#[test]
fn file_store_checks_what_it_reads_back() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("devices.json");
    let wrong = FileStore { path: p.clone(), read: Some(Arc::new(|_| Ok(b"something else".to_vec()))) };
    assert!(wrong.save(b"x").unwrap_err().contains("read back"), "a differing read-back must fail");
    let broken = FileStore { path: p, read: Some(Arc::new(|_| Err(std::io::Error::other("gone")))) };
    assert!(broken.save(b"x").is_err(), "an unreadable read-back must fail");
}

#[test]
fn secrets_and_ids_need_working_randomness() {
    assert!(new_id(&mut fail_rand()).is_err());
    assert!(new_secret(&mut fail_rand()).is_err());
    let id = new_id(&mut counter_rand(0)).unwrap();
    assert!(valid_id(&id), "{id}");
    let s = new_secret(&mut counter_rand(0)).unwrap();
    assert!(valid_secret(&s), "{s}");
    let (a, b) = (new_secret(&mut os_rand()).unwrap(), new_secret(&mut os_rand()).unwrap());
    assert!(valid_secret(&a) && a != b, "the operating system's source gives different secrets");
}

#[test]
fn the_ledger_file_uses_readable_state_names_that_do_not_follow_the_names_in_the_code() {
    let h = harness();
    let issued = h.add("one");
    assert_eq!(h.stored_state("one"), "issued");
    h.request(&issued, "11111111"); // waiting for the owner: a waiting request is never stored, the record stays issued
    assert!(h.stored_state("one") == "issued" && !String::from_utf8_lossy(&h.store.data().unwrap()).contains("requested"));
    h.granted("two", "22222222");
    assert_eq!(h.stored_state("two"), "granted");
    let three = h.active("three", "33333333").0;
    assert_eq!(h.stored_state("three"), "active");
    h.reg.revoke(&three).unwrap();
    assert_eq!(h.stored_state("three"), "revoked");
    h.add("four");
    h.clock.advance(h.cfg().ttl);
    h.reg.tick();
    assert_eq!(h.stored_state("four"), "expired");
    // what is stored is what a later version must keep reading
    for code in [STATE_ISSUED, STATE_GRANTED, STATE_ACTIVE, STATE_EXPIRED, STATE_REVOKED] {
        assert_eq!(state_code(code_state(code).unwrap()), code, "{code} is not a round trip");
    }
    assert!(code_state("requested").is_none(), "a waiting request is not a stored state");
}

// ---- adding ----

#[test]
fn add_issues_a_record_with_only_b() {
    let h = harness();
    let id = h.add("Nomad");
    assert!(valid_id(&id), "{id}");
    let (b, k) = h.reg.secrets(&id).expect("secrets");
    assert!(valid_secret(&b) && k.is_empty(), "want B only");
    let recs = h.ledger();
    assert!(recs.len() == 1 && recs[0].id == id && recs[0].state == STATE_ISSUED && recs[0].generation == 1 && recs[0].b == b && recs[0].k.is_empty(), "{recs:?}");
    let list = h.reg.list();
    assert!(list.len() == 1 && list[0].status == "issued" && list[0].expires == h.clock.now().add(h.cfg().ttl), "{list:?}");
}

#[test]
fn add_refuses_bad_names_duplicates_and_a_full_ledger() {
    let h = harness();
    for name in ["", &"あ".repeat(65), "tab\there"] {
        assert_eq!(h.reg.add(name), Err(Error::BadName), "add({name:?})");
    }
    h.add("one");
    assert_eq!(h.reg.add("one"), Err(Error::NameTaken));
    for i in 1..MAX_DEVICES {
        h.add(&format!("dev-{}", "x".repeat(i)));
    }
    let saves = h.store.saves();
    assert_eq!(h.reg.add("one too many"), Err(Error::Full));
    assert!(h.reg.list().len() == MAX_DEVICES && h.store.saves() == saves, "a refused Add changed the ledger");
}

#[test]
fn add_survives_an_id_collision_but_not_four_in_a_row() {
    let queue = Arc::new(Mutex::new(Vec::new()));
    let h = harness_rand(script_rand(queue.clone()), |_| {});
    let one = h.add("one"); // takes bytes 0..7 as its id
    assert_eq!(one, "0001020304050607", "test assumption");
    let first: Vec<u8> = (0..ID_BYTES as u8).collect();
    *lock(&queue) = vec![first.clone(), first.clone()]; // two collisions, then a fresh id
    let two = h.add("two");
    assert_ne!(two, one, "id collision was not resolved");
    *lock(&queue) = vec![first.clone(), first.clone(), first.clone(), first];
    match h.reg.add("three") {
        Err(Error::Other(m)) => assert!(m.contains("free device id"), "{m}"),
        other => panic!("four collisions: {other:?}"),
    }
    assert_eq!(h.reg.list().len(), 2, "a failed Add left a record behind");
}

#[test]
fn add_rolls_back_when_randomness_or_the_ledger_fails() {
    let h = harness_rand(fail_rand(), |_| {});
    assert!(h.reg.add("x").is_err(), "id: no randomness should fail");
    let h = harness_rand(after_n(ID_BYTES), |_| {}); // the id comes first; make only the secret fail
    assert!(h.reg.add("x").is_err(), "B: no randomness should fail");
    assert!(h.reg.list().is_empty(), "a failed Add left a record behind");
    let h = harness();
    h.store.fail_saves(Some("disk full"));
    assert!(h.reg.add("x").is_err() && h.reg.list().is_empty() && h.store.data().is_none(), "a ledger that cannot be written must not gain a record");
}

#[test]
fn a_date_the_ledger_cannot_write_is_refused_and_rolled_back() {
    let h = harness();
    h.clock.set(Timestamp::ymd_hms(10000, 1, 1, 0, 0, 0)); // JSON cannot hold a year above 9999
    assert!(h.reg.add("Nomad").is_err(), "encoding must fail");
    assert!(h.reg.list().is_empty(), "a record that cannot be stored must not stay");
}

// ---- pairing ----

#[test]
fn pairing_from_request_to_active() {
    let h = harness();
    let id = h.add("Nomad");
    let conn = h.request(&id, "12345678");
    h.must_status(&id, "waiting");

    assert_eq!(h.reg.confirm("12345678").as_deref(), Ok("Nomad"));
    h.must_status(&id, "granting");
    let grants = conn.grants();
    assert!(grants.len() == 1 && valid_secret(&grants[0].0), "{grants:?}");
    let k = grants[0].0.clone();
    assert_eq!(grants[0].1, vec![format!("wss://bridge.example/v1?d={id}")], "endpoints sent with K");
    assert_eq!(h.hooks.lock().unwrap().endpoints, vec![(id.clone(), k.clone())]);
    // K must be in the ledger before the device is told (a crash in between must not lose it).
    let at_grant = decode(&conn.at_grant.lock().unwrap().clone().unwrap()).unwrap().0;
    assert!(at_grant.len() == 1 && at_grant[0].state == STATE_GRANTED && at_grant[0].k == k && !at_grant[0].b.is_empty(), "ledger at the moment of the grant: {at_grant:?}");
    let (b, k2) = h.reg.secrets(&id).unwrap();
    assert!(!b.is_empty() && k2 == k, "while granting both secrets are valid");

    h.reg.ack(&id, h.gen_of(&id)).unwrap();
    h.must_status(&id, "active");
    let (b, k2) = h.reg.secrets(&id).unwrap();
    assert!(b.is_empty() && k2 == k, "after the ack B is gone and K stays");
    let rec = &h.ledger()[0];
    assert!(rec.state == STATE_ACTIVE && rec.b.is_empty() && rec.k == k, "{rec:?}");
    assert!(h.changed() > 0, "the relay was not told the secrets changed");
}

#[test]
fn a_connection_with_k_counts_as_the_ack() {
    let h = harness();
    let (id, _) = h.granted("Nomad", "12345678");
    h.reg.connect_k(&id).unwrap();
    h.must_status(&id, "active");
    assert!(h.reg.secrets(&id).unwrap().0.is_empty(), "B must be gone once K is in use");
}

#[test]
fn connect_k_errors() {
    let h = harness();
    assert_eq!(h.reg.connect_k("0000000000000000"), Err(Error::UnknownDevice));
    let id = h.add("Nomad");
    assert_eq!(h.reg.connect_k(&id), Err(Error::NotEligible), "K before it exists");
}

#[test]
fn connecting_with_k_only_writes_last_seen_once_per_minute() {
    let h = harness();
    let (id, _) = h.active("Nomad", "12345678");
    let saves = h.store.saves();
    h.clock.advance(LAST_SEEN_EVERY - Duration::from_secs(1));
    h.reg.connect_k(&id).unwrap();
    assert_eq!(h.store.saves(), saves, "a connection within the minute rewrote the ledger");
    h.clock.advance(Duration::from_secs(2));
    h.reg.connect_k(&id).unwrap();
    assert!(h.store.saves() == saves + 1 && !h.ledger()[0].last_seen.is_zero(), "a connection after the minute must store the time");
}

#[test]
fn ack_only_counts_for_the_right_generation() {
    let h = harness();
    let (id, _) = h.granted("Nomad", "12345678");
    assert_eq!(h.reg.ack(&id, h.gen_of(&id) + 1), Err(Error::UnknownDevice), "wrong generation");
    assert_eq!(h.reg.ack("ffffffffffffffff", 1), Err(Error::UnknownDevice));
    h.must_status(&id, "granting");
    // an ack in a state that is not granting is a late or repeated frame: nothing happens
    let other = h.add("Other");
    assert_eq!(h.reg.ack(&other, 1), Ok(()), "stray ack");
    h.must_status(&other, "issued");
}

#[test]
fn request_validates_what_it_is_given() {
    let h = harness();
    let id = h.add("Nomad");
    for (rid, code, generation) in [("bad", "12345678", 1), (id.as_str(), "1234567", 1), (id.as_str(), "1234567a", 1), (id.as_str(), "12345678", 0)] {
        let conn = Arc::new(FakeConn::default());
        assert_eq!(h.reg.request(rid, generation, code, conn.clone()), Err(Error::BadRequest), "{rid} {code} {generation}");
        assert_eq!(conn.reasons(), DENY_UNKNOWN_DEVICE);
    }
    let conn = Arc::new(FakeConn::default());
    assert_eq!(h.reg.request("ffffffffffffffff", 1, "12345678", conn.clone()), Err(Error::UnknownDevice));
    assert_eq!(conn.reasons(), DENY_UNKNOWN_DEVICE, "unknown device");
    let conn = Arc::new(FakeConn::default());
    assert_eq!(h.reg.request(&id, 2, "12345678", conn.clone()), Err(Error::UnknownDevice));
    assert_eq!(conn.reasons(), DENY_UNKNOWN_DEVICE, "old package (wrong generation)");
    h.must_status(&id, "issued");
}

#[test]
fn a_request_from_a_device_that_cannot_pair_is_refused() {
    let h = harness();
    let active = h.active("A", "11111111").0;
    let revoked = h.add("R");
    h.reg.revoke(&revoked).unwrap();
    for id in [active, revoked] {
        let conn = Arc::new(FakeConn::default());
        assert_eq!(h.reg.request(&id, h.gen_of(&id), "22222222", conn.clone()), Ok(()), "{id}");
        assert_eq!(conn.reasons(), DENY_UNKNOWN_DEVICE, "{id}");
    }
    assert!(h.logs().contains("devices.never"), "the impossible request was not recorded");
}

#[test]
fn a_request_while_granting_is_refused() {
    let h = harness();
    let (id, _) = h.granted("Nomad", "12345678");
    let conn = Arc::new(FakeConn::default());
    assert_eq!(h.reg.request(&id, h.gen_of(&id), "87654321", conn.clone()), Ok(()));
    assert_eq!(conn.reasons(), DENY_GRANT_PENDING);
    h.must_status(&id, "granting");
}

#[test]
fn the_same_code_twice_cancels_both_and_never_grants_either() {
    let h = harness();
    let id = h.add("Nomad");
    let first = h.request(&id, "12345678");
    let second = h.request(&id, "12345678"); // someone who saw the code on the screen
    assert!(first.reasons() == DENY_CODE_IN_USE && second.reasons() == DENY_CODE_IN_USE, "first {:?} second {:?}", first.reasons(), second.reasons());
    h.must_status(&id, "issued"); // nobody is left waiting
    assert_eq!(h.reg.confirm("12345678"), Err(Error::NoPending), "Confirm after the taint");
}

#[test]
fn the_same_code_among_several_requests_keeps_the_others() {
    let h = harness();
    let id = h.add("Nomad");
    let a = h.request(&id, "11111111");
    let b = h.request(&id, "22222222");
    let c = h.request(&id, "11111111");
    assert!(a.reasons() == DENY_CODE_IN_USE && c.reasons() == DENY_CODE_IN_USE && b.reasons().is_empty());
    h.must_status(&id, "waiting");
    assert!(h.reg.confirm("22222222").is_ok() && b.grants().len() == 1, "the untouched request must still be grantable");
}

#[test]
fn too_many_waiting_requests_are_refused() {
    let h = harness_with(|c| c.max_pending = 2);
    let id = h.add("Nomad");
    h.request(&id, "11111111");
    h.request(&id, "22222222");
    let third = h.request(&id, "33333333");
    assert_eq!(third.reasons(), DENY_PAIR_BUSY);
    h.must_status(&id, "waiting");
    assert_eq!(h.reg.list()[0].waiting, 2);
}

#[test]
fn confirm_refuses_what_it_cannot_use() {
    let h = harness();
    assert_eq!(h.reg.confirm("12345"), Err(Error::BadCode), "short code");
    assert_eq!(h.reg.confirm("12345678"), Err(Error::NoPending), "nobody waiting");
    let id = h.add("Nomad");
    h.request(&id, "12345678");
    assert_eq!(h.reg.confirm("00000000"), Err(Error::CodeRejected), "wrong code");
    h.must_status(&id, "waiting");
}

#[test]
fn five_wrong_codes_close_the_window() {
    let h = harness();
    let id = h.add("Nomad");
    let conn = h.request(&id, "12345678");
    for i in 0..4 {
        assert_eq!(h.reg.confirm("00000000"), Err(Error::CodeRejected), "attempt {}", i + 1);
        h.must_status(&id, "waiting");
    }
    assert_eq!(h.reg.confirm("00000000"), Err(Error::CodeRejected));
    h.must_status(&id, "issued");
    assert_eq!(conn.reasons(), DENY_WINDOW_CLOSED, "the waiting request was told");
    // the right code no longer works, but a new request opens a fresh window with a fresh count
    assert_eq!(h.reg.confirm("12345678"), Err(Error::NoPending), "after the window closed");
    h.request(&id, "87654321");
    assert!(h.reg.confirm("87654321").is_ok(), "a new window starts with no failures");
}

#[test]
fn a_code_that_two_devices_show_is_ambiguous_and_grants_neither() {
    let h = harness();
    let (one, two) = (h.add("one"), h.add("two"));
    let (c1, c2) = (h.request(&one, "12345678"), h.request(&two, "12345678"));
    assert_eq!(h.reg.confirm("12345678"), Err(Error::CodeRejected), "ambiguous code");
    assert_eq!(c1.grants().len() + c2.grants().len(), 0, "an ambiguous code granted a device");
    h.must_status(&one, "waiting");
    h.must_status(&two, "waiting");
}

#[test]
fn confirm_picks_the_right_device_among_several() {
    let h = harness();
    let (one, two) = (h.add("one"), h.add("two"));
    let (c1, c2) = (h.request(&one, "11111111"), h.request(&two, "22222222"));
    assert_eq!(h.reg.confirm("22222222").as_deref(), Ok("two"));
    assert!(c1.grants().is_empty() && c2.grants().len() == 1);
    h.must_status(&one, "waiting");
}

#[test]
fn granting_is_undone_when_the_ledger_cannot_be_written() {
    let h = harness();
    let id = h.add("Nomad");
    let conn = h.request(&id, "12345678");
    h.store.fail_saves(Some("disk full"));
    assert!(h.reg.confirm("12345678").is_err(), "Confirm must fail");
    h.must_status(&id, "waiting");
    assert!(conn.grants().is_empty(), "K was sent although it was not stored");
    let (b, k) = h.reg.secrets(&id).unwrap();
    assert!(!b.is_empty() && k.is_empty(), "secrets after the rollback");
    h.store.fail_saves(None); // the owner can try again
    assert!(h.reg.confirm("12345678").is_ok() && conn.grants().len() == 1, "retry");
}

#[test]
fn granting_fails_cleanly_without_randomness() {
    let h = harness();
    let id = h.add("Nomad");
    let conn = h.request(&id, "12345678");
    h.set_rand(fail_rand());
    assert!(h.reg.confirm("12345678").is_err() && conn.grants().is_empty());
    h.must_status(&id, "waiting");
}

#[test]
fn a_grant_that_cannot_be_sent_returns_the_device_to_issued() {
    let h = harness();
    let id = h.add("Nomad");
    let conn = Arc::new(FakeConn { grant_err: Some("connection reset".into()), store: Some(h.store.clone()), ..FakeConn::default() });
    h.reg.request(&id, 1, "12345678", conn).unwrap();
    h.reg.confirm("12345678").unwrap();
    h.must_status(&id, "issued");
    assert!(h.reg.secrets(&id).unwrap().1.is_empty(), "K must be dropped when it could not be sent");
    let rec = &h.ledger()[0];
    assert!(rec.state == STATE_ISSUED && rec.k.is_empty(), "{rec:?}");
}

#[test]
fn disconnect_drops_only_that_request() {
    let h = harness();
    let id = h.add("Nomad");
    let (a, b) = (h.request(&id, "11111111"), h.request(&id, "22222222"));
    let (ac, bc): (Arc<dyn Conn>, Arc<dyn Conn>) = (a, b);
    h.reg.disconnect(&ac);
    h.must_status(&id, "waiting");
    assert_eq!(h.reg.confirm("11111111"), Err(Error::CodeRejected), "the dropped request must not be confirmable");
    h.reg.disconnect(&bc);
    h.must_status(&id, "issued");
    h.reg.disconnect(&(Arc::new(FakeConn::default()) as Arc<dyn Conn>)); // a connection nobody knows
    h.must_status(&id, "issued");
}

#[test]
fn the_window_closes_by_itself() {
    let h = harness();
    let id = h.add("Nomad");
    let conn = h.request(&id, "12345678");
    h.clock.advance(h.cfg().window - Duration::from_secs(1));
    h.reg.tick();
    h.must_status(&id, "waiting");
    h.clock.advance(Duration::from_secs(1));
    h.reg.tick();
    h.must_status(&id, "issued");
    assert_eq!(conn.reasons(), DENY_WINDOW_CLOSED);
    assert_eq!(h.reg.confirm("12345678"), Err(Error::NoPending), "after the window");
}

#[test]
fn an_unconfirmed_grant_falls_back_to_issued() {
    let h = harness();
    let (id, _) = h.granted("Nomad", "12345678");
    h.clock.advance(h.cfg().ack_timeout);
    h.reg.tick();
    h.must_status(&id, "issued");
    let (b, k) = h.reg.secrets(&id).unwrap();
    assert!(!b.is_empty() && k.is_empty(), "B must stay and K must go");
    h.request(&id, "87654321"); // the device may ask again
    h.must_status(&id, "waiting");
}

#[test]
fn b_expires_after_its_time_to_live() {
    // the window is longer than the TTL here, so the TTL is what ends the waiting
    let h = harness_with(|c| {
        c.ttl = Duration::from_secs(60);
        c.window = Duration::from_secs(3600);
    });
    let (issued, queued) = (h.add("issued"), h.add("queued"));
    let conn = h.request(&queued, "12345678");
    h.clock.advance(h.cfg().ttl);
    h.reg.tick();
    h.must_status(&issued, "expired");
    h.must_status(&queued, "expired");
    assert_eq!(conn.reasons(), DENY_EXPIRED, "waiting request told");
    assert!(h.reg.secrets(&issued).is_none(), "an expired device still has a valid secret");
    let rec = &h.ledger()[0];
    assert!(rec.state == STATE_EXPIRED && rec.b.is_empty(), "{rec:?}");
}

#[test]
fn a_grant_in_progress_outlives_the_ttl_and_then_expires() {
    let h = harness_with(|c| c.ttl = Duration::from_secs(60));
    let (id, _) = h.granted("Nomad", "12345678");
    h.clock.advance(Duration::from_secs(10));
    h.reg.tick();
    h.must_status(&id, "granting"); // TTL has not passed yet
    h.clock.advance(Duration::from_secs(60));
    h.reg.tick(); // TTL and the ack timeout are both due: the ack timeout returns it to issued, which expires at once
    h.must_status(&id, "expired");
    assert!(h.reg.secrets(&id).is_none(), "expired device keeps a secret");
}

#[test]
fn a_window_that_ends_after_the_ttl_leaves_the_device_expired() {
    let h = harness_with(|c| {
        c.ttl = Duration::from_secs(3600);
        c.window = Duration::from_secs(7200);
    });
    let id = h.add("Nomad");
    h.request(&id, "12345678");
    h.clock.advance(Duration::from_secs(3 * 3600));
    h.reg.tick();
    h.must_status(&id, "expired");
}

#[test]
fn an_active_device_never_expires() {
    let h = harness();
    let (id, _) = h.active("Nomad", "12345678");
    h.clock.advance(Duration::from_secs(1000 * 3600));
    h.reg.tick();
    h.must_status(&id, "active");
}

// ---- the owner's operations ----

#[test]
fn revoke_removes_both_secrets_and_ends_every_session() {
    let h = harness();
    let (id, _) = h.active("Nomad", "12345678");
    h.reg.revoke(&id).unwrap();
    h.must_status(&id, "revoked");
    assert!(h.reg.secrets(&id).is_none(), "a revoked device keeps a secret");
    assert_eq!(h.closed(), vec![id.clone()], "sessions closed");
    let rec = &h.ledger()[0];
    assert!(rec.state == STATE_REVOKED && rec.k.is_empty() && !rec.revoked_at.is_zero(), "{rec:?}");
    assert_eq!(h.reg.revoke(&id), Err(Error::AlreadyRevoked), "second revoke");
    assert_eq!(h.reg.revoke("ffffffffffffffff"), Err(Error::UnknownDevice));
}

#[test]
fn revoke_tells_waiting_requests() {
    let h = harness();
    let id = h.add("Nomad");
    let conn = h.request(&id, "12345678");
    assert_eq!(h.reg.revoke(&id), Ok(()));
    assert_eq!(conn.reasons(), DENY_UNKNOWN_DEVICE);
}

#[test]
fn revoke_holds_even_when_the_ledger_cannot_be_written() {
    let h = harness();
    let (id, _) = h.active("Nomad", "12345678");
    h.store.fail_saves(Some("disk full"));
    assert!(matches!(h.reg.revoke(&id), Err(Error::NotPersisted(_))));
    h.must_status(&id, "revoked"); // kept in memory: a revoked device must not be served
    assert_eq!(h.closed().len(), 1, "sessions must be closed even though the write failed");
    assert_eq!(h.ledger()[0].state, STATE_ACTIVE, "the file still says active: the owner has been told it was not saved");
}

#[test]
fn reissue_gives_a_new_b_and_drops_k() {
    let h = harness();
    let (id, _) = h.active("Nomad", "12345678");
    let old_b = h.ledger()[0].b.clone();
    h.reg.reissue(&id).unwrap();
    h.must_status(&id, "issued");
    let (b, k) = h.reg.secrets(&id).unwrap();
    assert!(!b.is_empty() && b != old_b && k.is_empty() && h.gen_of(&id) == 2, "B {b:?} K {k:?}");
    assert_eq!(h.closed(), vec![id.clone()], "the old K's sessions must end");
    // the old package (generation 1) is refused
    assert_eq!(h.reg.request(&id, 1, "11111111", Arc::new(FakeConn::default())), Err(Error::UnknownDevice), "old generation");
}

#[test]
fn reissue_from_expired_and_from_waiting() {
    let h = harness();
    let expired = h.add("expired");
    h.clock.advance(h.cfg().ttl);
    h.reg.tick();
    h.reg.reissue(&expired).unwrap();
    h.must_status(&expired, "issued");
    assert!(h.reg.secrets(&expired).is_some_and(|(b, _)| !b.is_empty()), "a reissued device must have a B again");
    let waiting = h.add("waiting");
    let conn = h.request(&waiting, "12345678");
    assert_eq!(h.reg.reissue(&waiting), Ok(()));
    assert_eq!(conn.reasons(), DENY_SUPERSEDED);
}

#[test]
fn reissue_refusals_and_rollback() {
    let h = harness();
    let (id, _) = h.active("Nomad", "12345678");
    assert_eq!(h.reg.reissue("ffffffffffffffff"), Err(Error::UnknownDevice));
    h.set_rand(fail_rand());
    assert!(h.reg.reissue(&id).is_err(), "no randomness should fail");
    h.set_rand(counter_rand(200));
    h.store.fail_saves(Some("disk full"));
    assert!(h.reg.reissue(&id).is_err(), "an unwritable ledger should fail");
    h.must_status(&id, "active");
    assert!(h.gen_of(&id) == 1 && h.closed().is_empty(), "a failed reissue must change nothing");
    h.store.fail_saves(None);
    h.reg.revoke(&id).unwrap();
    assert_eq!(h.reg.reissue(&id), Err(Error::Revoked), "a revoked device cannot be reissued");
}

#[test]
fn forget_only_deletes_revoked_or_expired_records() {
    let h = harness();
    let active = h.active("A", "11111111").0;
    assert_eq!(h.reg.forget(&active), Err(Error::StillValid));
    assert_eq!(h.reg.forget("ffffffffffffffff"), Err(Error::UnknownDevice));
    h.reg.revoke(&active).unwrap();
    let expired = h.add("X");
    h.clock.advance(h.cfg().ttl);
    h.reg.tick();
    for id in [&active, &expired] {
        h.reg.forget(id).unwrap_or_else(|e| panic!("forget({id}): {e}"));
    }
    assert!(h.reg.list().is_empty() && h.ledger().is_empty(), "forgotten records are still there");
    h.add("A"); // the name is free again
}

#[test]
fn forget_is_undone_when_the_ledger_cannot_be_written() {
    let h = harness();
    let id = h.add("one");
    h.add("two");
    h.reg.revoke(&id).unwrap();
    h.store.fail_saves(Some("disk full"));
    assert!(h.reg.forget(&id).is_err(), "Forget must fail");
    let list = h.reg.list();
    assert!(list.len() == 2 && list[0].id == id, "the record must come back where it was: {list:?}");
}

#[test]
fn repack_changes_nothing() {
    let h = harness();
    let id = h.add("Nomad");
    let saves = h.store.saves();
    let d = h.reg.repack(&id).unwrap();
    assert!(d.id == id && d.name == "Nomad" && d.generation == 1 && valid_secret(&d.b) && d.window == h.cfg().window, "{d:?}");
    h.request(&id, "12345678");
    assert!(!h.reg.repack(&id).unwrap().b.is_empty(), "while waiting the package still carries B");
    h.reg.confirm("12345678").unwrap();
    assert!(!h.reg.repack(&id).unwrap().b.is_empty(), "while granting the package still carries B");
    h.reg.ack(&id, 1).unwrap();
    assert!(h.reg.repack(&id).unwrap().b.is_empty(), "after pairing the package carries no secret");
    assert_eq!(h.store.saves(), saves + 2, "only the confirm (granted) and the ack (active) write; Repack and a request write nothing");
}

#[test]
fn repack_refusals() {
    let h = harness();
    assert_eq!(h.reg.repack("ffffffffffffffff"), Err(Error::UnknownDevice));
    let expired = h.add("X");
    h.clock.advance(h.cfg().ttl);
    h.reg.tick();
    assert_eq!(h.reg.repack(&expired), Err(Error::Expired));
    let revoked = h.add("R");
    h.reg.revoke(&revoked).unwrap();
    assert_eq!(h.reg.repack(&revoked), Err(Error::Revoked));
}

#[test]
fn active_secrets_lists_every_valid_secret() {
    let h = harness();
    let issued = h.add("issued");
    let granted = h.granted("granted", "11111111").0;
    let active = h.active("active", "22222222").0;
    let revoked = h.add("revoked");
    h.reg.revoke(&revoked).unwrap();
    let mut got: Vec<String> = h
        .reg
        .active_secrets()
        .into_iter()
        .map(|s| {
            assert!(valid_secret(&s.secret), "bad secret for {}", s.id);
            format!("{}/{}", s.id, if s.is_k { "K" } else { "B" })
        })
        .collect();
    let mut want = vec![format!("{issued}/B"), format!("{granted}/B"), format!("{granted}/K"), format!("{active}/K")];
    got.sort();
    want.sort();
    assert_eq!(got, want);
}

#[test]
fn secrets_of_an_unknown_device() {
    assert!(harness().reg.secrets("ffffffffffffffff").is_none());
}

#[test]
fn changed_is_called_only_when_something_changed() {
    let h = harness();
    let id = h.add("Nomad");
    let n = h.changed();
    h.reg.tick(); // nothing is due
    h.reg.disconnect(&(Arc::new(FakeConn::default()) as Arc<dyn Conn>));
    assert!(h.reg.confirm("12345678").is_err(), "nobody is waiting");
    assert_eq!(h.changed(), n, "Changed was called for no change");
    h.request(&id, "12345678");
    assert_ne!(h.changed(), n, "a new request changes the state: Changed expected");
}

#[test]
fn the_registry_works_without_hooks_and_with_defaults() {
    let (r, _) = Registry::new(Config::default(), Box::new(MemStore::default()), Hooks::default(), os_rand()).unwrap();
    let id = r.add("Nomad").unwrap();
    let c = Arc::new(FakeConn::default());
    r.request(&id, 1, "12345678", c.clone()).unwrap();
    assert!(r.confirm("12345678").is_ok() && c.grants().len() == 1 && c.grants()[0].1.is_empty(), "no Endpoints hook: no endpoints");
    r.revoke(&id).unwrap();
    let cfg = Config::default().with_defaults();
    assert!(cfg.window == Duration::from_secs(90) && cfg.ttl == Duration::from_secs(24 * 3600) && cfg.ack_timeout == Duration::from_secs(30) && cfg.max_pending == 4 && cfg.max_attempts == 5);
}

#[test]
fn the_registry_is_safe_for_concurrent_use() {
    let h = harness();
    let id = h.add("Nomad");
    let reg = h.reg.clone();
    let threads: Vec<_> = (0..8)
        .map(|i| {
            let (reg, id) = (reg.clone(), id.clone());
            std::thread::spawn(move || {
                for _ in 0..50 {
                    let c: Arc<dyn Conn> = Arc::new(FakeConn::default());
                    let _ = reg.request(&id, 1, &format!("1234567{i}"), c.clone());
                    reg.disconnect(&c);
                    reg.list();
                    reg.tick();
                    reg.secrets(&id);
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
}

#[test]
fn confirming_one_request_tells_the_others_they_were_superseded() {
    let h = harness();
    let id = h.add("Nomad");
    let (a, b) = (h.request(&id, "11111111"), h.request(&id, "22222222"));
    h.reg.confirm("22222222").unwrap();
    assert!(a.reasons() == DENY_SUPERSEDED && b.grants().len() == 1 && b.reasons().is_empty());
}

#[test]
fn an_old_grant_failure_does_not_touch_a_newer_state() {
    let h = harness();
    let (id, _) = h.granted("Nomad", "12345678");
    let generation = h.gen_of(&id);
    h.reg.locked(|i| i.timeout_grant("ffffffffffffffff", generation)).expect("unknown device");
    h.reg.locked(|i| i.timeout_grant(&id, generation + 1)).expect("another generation");
    h.must_status(&id, "granting");
    h.reg.ack(&id, generation).unwrap();
    h.reg.locked(|i| i.timeout_grant(&id, generation)).unwrap();
    h.must_status(&id, "active"); // a late failure report must not undo an acknowledged grant
}

// A write that fails after an action that normally writes nothing: these tests force the ledger to
// differ from what is stored so that the error from the store reaches the caller.
#[test]
fn store_errors_reach_the_caller() {
    let h = harness();
    let id = h.add("Nomad");
    h.request(&id, "12345678");
    h.reg.locked(|i| i.saved = Some(b"stale".to_vec()));
    h.store.fail_saves(Some("disk full"));
    assert!(matches!(h.reg.confirm("00000000"), Err(e) if e != Error::CodeRejected), "Confirm with a failing store");
    let h2 = harness();
    let id2 = h2.add("Nomad");
    h2.reg.locked(|i| i.saved = Some(b"stale".to_vec()));
    h2.store.fail_saves(Some("disk full"));
    assert!(h2.reg.repack(&id2).is_err(), "Repack with a failing store");
}

#[test]
fn confirm_for_looks_only_at_the_named_device() {
    let h = harness();
    let (one, two) = (h.add("one"), h.add("two"));
    let (c1, c2) = (h.request(&one, "11111111"), h.request(&two, "22222222"));
    // the code of the other device, with this device's id: not accepted, and only this device is charged
    assert_eq!(h.reg.confirm_for(&one, "22222222"), Err(Error::CodeRejected), "a code of another device");
    assert_eq!(c1.grants().len() + c2.grants().len(), 0, "a code with the wrong id granted a device");
    assert_eq!(h.reg.confirm_for(&two, "22222222").as_deref(), Ok("two"));
    assert_eq!(c2.grants().len(), 1);
    h.must_status(&one, "waiting");
}

#[test]
fn confirm_for_counts_wrong_codes_against_that_device_only() {
    let h = harness();
    let (one, two) = (h.add("one"), h.add("two"));
    let c1 = h.request(&one, "11111111");
    h.request(&two, "22222222");
    for _ in 0..5 {
        let _ = h.reg.confirm_for(&one, "00000000");
    }
    h.must_status(&one, "issued"); // its window is closed
    assert_eq!(c1.reasons(), DENY_WINDOW_CLOSED);
    h.must_status(&two, "waiting"); // the other device is untouched
}

#[test]
fn confirm_for_refusals() {
    let h = harness();
    let id = h.add("Nomad");
    assert_eq!(h.reg.confirm_for("bad", "12345678"), Err(Error::UnknownDevice), "a malformed id");
    assert_eq!(h.reg.confirm_for("ffffffffffffffff", "12345678"), Err(Error::UnknownDevice), "an unknown device");
    assert_eq!(h.reg.confirm_for(&id, "1"), Err(Error::BadCode), "a short code");
    assert_eq!(h.reg.confirm_for(&id, "12345678"), Err(Error::NoPending), "a device that shows no code");
}

// ---- the table ----

#[test]
fn every_cell_exists_and_every_action_has_a_handler() {
    // The match in table::cell is exhaustive, so a missing cell does not compile; run_action is an
    // exhaustive match on DevAction too. What is left to check is that each action is reachable.
    use crate::devices::table::{Cell, cell};
    let mut used = std::collections::HashSet::new();
    for s in DevState::ALL {
        for e in DevEvent::ALL {
            match cell(*s, *e) {
                Cell::Do { action, .. } | Cell::IgnoreDo(action) => {
                    used.insert(action);
                }
                Cell::Guard { more, spent } => {
                    used.insert(more.action);
                    used.insert(spent.action);
                }
                _ => {}
            }
        }
    }
    for a in DevAction::ALL {
        assert!(used.contains(a), "action {} is in no cell", a.name());
    }
}

/// A record that is in state `s`, made by doing what a real device does.
fn record_in(h: &Harness, s: DevState) -> Rec {
    let id = match s {
        DevState::NoRecord => return Rec::new(Record::default(), DevState::NoRecord),
        DevState::Issued => h.add("dev-I"),
        DevState::Requested => {
            let id = h.add("dev-Q");
            h.request(&id, "11111111");
            id
        }
        DevState::Granted => h.granted("dev-G", "11111111").0,
        DevState::Active => h.active("dev-A", "11111111").0,
        DevState::Expired => {
            let id = h.add("dev-X");
            h.clock.advance(h.cfg().ttl);
            h.reg.tick();
            id
        }
        DevState::Revoked => {
            let id = h.add("dev-R");
            h.reg.revoke(&id).unwrap();
            id
        }
    };
    h.reg.locked(|i| i.recs.get(&id).cloned().expect("the record"))
}

fn arg_for(rc: &Rec, e: DevEvent) -> Arg {
    let some_conn: Arc<dyn Conn> = Arc::new(FakeConn::default());
    match e {
        DevEvent::OwnerAdds => Arg::Name("another-name".into()),
        DevEvent::DeviceRequests => Arg::Request { code: "22222222".into(), conn: Arc::new(FakeConn::default()) },
        DevEvent::RequestDrops => Arg::Conn(rc.pending.first().map(|p| p.conn.clone()).unwrap_or(some_conn)),
        DevEvent::OwnerConfirms => Arg::Chosen(rc.pending.first().cloned().unwrap_or(Pending { code: "99999999".into(), conn: some_conn })),
        DevEvent::OwnerRepacks => Arg::Repack,
        _ => Arg::None,
    }
}

#[test]
fn every_cell_does_what_the_table_says() {
    use crate::devices::table::{Cell, cell};
    for s in DevState::ALL {
        for e in DevEvent::ALL {
            let c = cell(*s, *e);
            let branches: &[bool] = if matches!(c, Cell::Guard { .. }) { &[false, true] } else { &[false] };
            for &spent in branches {
                let h = harness();
                let rc = record_in(&h, *s);
                let name = format!("{} × {}", s.name(), e.name());
                let (before, before_state, before_pending) = (rc.record.clone(), rc.state, rc.pending.len());
                let id = rc.record.id.clone();
                let arg = arg_for(&rc, *e);
                let (out, r) = h.reg.locked(|i| {
                    i.recs.remove(&id);
                    i.fire_rec(rc, *e, arg, spent)
                });
                assert_eq!(r, Ok(()), "{name} (spent={spent})");
                let after = h.reg.locked(|i| if *s == DevState::NoRecord { i.order.last().and_then(|x| i.recs.get(x)).cloned() } else { i.recs.get(&id).cloned() });
                match c {
                    Cell::Never(_) => assert_eq!(out, Outcome::Never, "{name}"),
                    Cell::Ignore(_) | Cell::IgnoreDo(_) => assert_eq!(out, Outcome::Ignored, "{name}"),
                    _ => assert_eq!(out, Outcome::Moved, "{name}"),
                }
                if matches!(c, Cell::Never(_) | Cell::Ignore(_) | Cell::IgnoreDo(_)) {
                    if *s == DevState::NoRecord {
                        continue; // there is no record to compare
                    }
                    let after = after.expect("the record is still there");
                    assert!(after.state == before_state && after.record == before && after.pending.len() == before_pending, "{name}: a cell that does nothing changed the record");
                    continue;
                }
                let want = match c {
                    Cell::Do { next, .. } => next,
                    Cell::Guard { more, spent: sp } => {
                        if spent {
                            sp.next
                        } else {
                            more.next
                        }
                    }
                    _ => unreachable!(),
                };
                match after {
                    Some(a) => assert_eq!(a.state, want, "{name} (spent={spent})"),
                    // FORGET deletes the record: the table sends it to NoRecord
                    None => assert_eq!(want, DevState::NoRecord, "{name}: the record is gone but the table says {}", want.name()),
                }
            }
        }
    }
}

// ---- the document ----

fn render(state: DevState, c: crate::devices::table::Cell) -> String {
    use crate::devices::table::Cell;
    match c {
        Cell::Never(note) => format!("—{note}"),
        Cell::Ignore(note) => format!("×{note}"),
        Cell::IgnoreDo(a) => a.name().to_string(),
        Cell::Do { action, next } if next == state => action.name().to_string(),
        Cell::Do { action, next } => format!("{}→{}", action.name(), next.name()),
        Cell::Guard { more, spent } => format!("{}→{} [more] / {}→{} [spent]", more.action.name(), more.next.name(), spent.action.name(), spent.next.name()),
    }
}

/// The document is the source of the table; the generated code must say the same in every cell.
/// If this fails: change the document, then run `npm run spec` in plugin/.
#[test]
fn the_table_in_the_document_is_the_table_in_the_code() {
    let raw = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/spec/pairing.md")).expect("docs/spec/pairing.md");
    let (mut inside, mut header, mut seen): (bool, Option<Vec<DevEvent>>, usize) = (false, None, 0);
    for line in raw.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            inside = h.trim() == "記録: 状態遷移表";
            continue;
        }
        if !inside || !line.starts_with('|') {
            continue;
        }
        let cells: Vec<String> = line.trim_matches(|c| c == '|' || c == ' ').split('|').map(|c| c.trim().to_string()).collect();
        if cells[0].starts_with("状態＼イベント") {
            header = Some(cells[1..].iter().map(|e| *DevEvent::ALL.iter().find(|x| x.name() == e).unwrap_or_else(|| panic!("event {e} is in the document but not in the code"))).collect());
            continue;
        }
        let Some(header) = &header else { continue };
        if !cells[0].starts_with("**") {
            continue;
        }
        let sname = cells[0].trim_matches('*');
        let state = *DevState::ALL.iter().find(|x| x.name() == sname).unwrap_or_else(|| panic!("state {sname} is in the document but not in the code"));
        for (i, e) in header.iter().enumerate() {
            let c = crate::devices::table::cell(state, *e);
            let mut got = render(state, c);
            // a cell that stays in its state may be written with or without the arrow to itself
            if let crate::devices::table::Cell::Do { action, next } = c {
                if next == state && cells[i + 1] == format!("{}→{}", action.name(), state.name()) {
                    got = cells[i + 1].clone();
                }
            }
            assert_eq!(got, cells[i + 1], "{} × {}: the document says {:?}, the code says {:?}", state.name(), e.name(), cells[i + 1], got);
            seen += 1;
        }
    }
    assert_eq!(seen, DevState::ALL.len() * DevEvent::ALL.len(), "the document has {seen} cells, the table has {}", DevState::ALL.len() * DevEvent::ALL.len());
}

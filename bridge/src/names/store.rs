//! The name service's records: ID → public key, LAN address, and the short-lived TXT values of
//! ACME challenges. Every request is checked here, before anything is written (interfaces.md
//! I28, I29, I31).

use crate::namesproto::{self, AcmeReq, RegisterReq};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Mutex;

/// How long an ACME TXT value stays (a challenge takes seconds; a stale one must not linger).
pub const TXT_TTL_SECS: i64 = 600;
/// At most this many TXT values per ID (a retry may leave one behind).
pub const MAX_TXT: usize = 2;
/// A record nobody has refreshed for this long is forgotten (bridges refresh at every start).
pub const FORGET_AFTER_SECS: i64 = 180 * 86_400;
pub const NONCE_MEMORY_SECS: i64 = 2 * namesproto::MAX_SKEW_SECS + 60;
const MAX_NONCES: usize = 200_000;

#[derive(Debug, PartialEq, Eq)]
pub enum ApiError {
    BadRequest(&'static str),
    Unauthorized(&'static str),
    Forbidden(&'static str),
    NotFound,
    Conflict,
    TooMany(&'static str),
    Internal(String),
}

impl ApiError {
    pub fn status(&self) -> u16 {
        match self {
            ApiError::BadRequest(_) => 400,
            ApiError::Unauthorized(_) => 401,
            ApiError::Forbidden(_) => 403,
            ApiError::NotFound => 404,
            ApiError::Conflict => 409,
            ApiError::TooMany(_) => 429,
            ApiError::Internal(_) => 500,
        }
    }

    /// The short name sent to the caller. Internal details are not sent.
    pub fn name(&self) -> &str {
        match self {
            ApiError::BadRequest(s) | ApiError::Unauthorized(s) | ApiError::Forbidden(s) | ApiError::TooMany(s) => s,
            ApiError::NotFound => "unknown_id",
            ApiError::Conflict => "key_mismatch",
            ApiError::Internal(_) => "internal",
        }
    }
}

#[derive(Debug, Clone)]
struct Record {
    public_key: Vec<u8>,
    ip: Option<Ipv4Addr>,
    updated: i64,
    /// When the last accepted request for this ID came (not saved).
    last_request: i64,
    txt: Vec<(String, i64)>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedRecord {
    public_key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    ip: String,
    updated: i64,
}

#[derive(Serialize, Deserialize, Default)]
struct Saved {
    records: HashMap<String, SavedRecord>,
}

#[derive(Default)]
struct Inner {
    records: HashMap<String, Record>,
    nonces: HashMap<String, i64>,
    order: VecDeque<(i64, String)>,
}

pub struct Limits {
    pub max_ids: usize,
    /// The least time between two accepted requests for one ID.
    pub min_interval_secs: i64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_ids: 100_000, min_interval_secs: 2 }
    }
}

pub struct Names {
    inner: Mutex<Inner>,
    path: Option<PathBuf>,
    limits: Limits,
}

impl Names {
    pub fn new(path: Option<PathBuf>, limits: Limits) -> Names {
        Names { inner: Mutex::new(Inner::default()), path, limits }
    }

    /// Reads the saved records. A record that fails the checks is left out and reported.
    pub fn load(path: PathBuf, limits: Limits, now: i64) -> Result<(Names, Vec<String>), String> {
        let names = Names::new(Some(path.clone()), limits);
        let mut problems = Vec::new();
        match std::fs::read(&path) {
            Ok(bytes) => {
                let saved: Saved = serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
                let mut inner = names.inner.lock().unwrap();
                for (id, r) in saved.records {
                    let pk = namesproto::unb64(&r.public_key).filter(|pk| pk.len() == 32 && namesproto::id_of(pk) == id);
                    let ip = if r.ip.is_empty() { Some(None) } else { namesproto::publishable_ip(&r.ip).map(Some) };
                    match (pk, ip) {
                        (Some(public_key), Some(ip)) if namesproto::valid_id(&id) && now - r.updated < FORGET_AFTER_SECS => {
                            inner.records.insert(id, Record { public_key, ip, updated: r.updated, last_request: 0, txt: Vec::new() });
                        }
                        (Some(_), Some(_)) if namesproto::valid_id(&id) => {} // old: forgotten on purpose
                        _ => problems.push(format!("{}: record {:?} is not valid; left out", path.display(), id.chars().take(40).collect::<String>())),
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
        Ok((names, problems))
    }

    fn save(&self, inner: &Inner) -> Result<(), String> {
        let Some(path) = &self.path else { return Ok(()) };
        let saved = Saved {
            records: inner
                .records
                .iter()
                .map(|(id, r)| (id.clone(), SavedRecord { public_key: namesproto::b64(&r.public_key), ip: r.ip.map(|i| i.to_string()).unwrap_or_default(), updated: r.updated }))
                .collect(),
        };
        let bytes = serde_json::to_vec(&saved).map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            crate::util::create_private_dir(dir).map_err(|e| e.to_string())?;
        }
        crate::util::write_private_atomic(path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        let again = std::fs::read(path).map_err(|e| e.to_string())?;
        if again != bytes {
            return Err(format!("{}: reading the records back gave other bytes", path.display()));
        }
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().records.len()
    }

    /// Adds a record without a signature, for this process's own identity (it needs only the TXT
    /// record). Not saved: it is added again at every start.
    pub fn add_local(&self, id: &str, public_key: Vec<u8>, now: i64) {
        let mut inner = self.inner.lock().unwrap();
        inner.records.insert(id.to_string(), Record { public_key, ip: None, updated: now, last_request: 0, txt: Vec::new() });
    }

    fn check_time(time: i64, now: i64) -> Result<(), ApiError> {
        if (time - now).abs() > namesproto::MAX_SKEW_SECS { Err(ApiError::Unauthorized("bad_time")) } else { Ok(()) }
    }

    /// Remembers a nonce; false if it was already seen. Called after the signature is proven, so
    /// nobody without a key can fill the memory.
    fn remember_nonce(inner: &mut Inner, id: &str, nonce: &str, now: i64) -> Result<(), ApiError> {
        while let Some((at, key)) = inner.order.front().cloned() {
            if now - at <= NONCE_MEMORY_SECS {
                break;
            }
            inner.order.pop_front();
            inner.nonces.remove(&key);
        }
        let key = format!("{id}/{nonce}");
        if inner.nonces.contains_key(&key) {
            return Err(ApiError::Unauthorized("replayed"));
        }
        if inner.nonces.len() >= MAX_NONCES {
            return Err(ApiError::TooMany("busy"));
        }
        inner.nonces.insert(key.clone(), now);
        inner.order.push_back((now, key));
        Ok(())
    }

    pub fn register(&self, req: &RegisterReq, now: i64) -> Result<(), ApiError> {
        if !namesproto::valid_id(&req.id) {
            return Err(ApiError::BadRequest("bad_id"));
        }
        let public_key = namesproto::unb64(&req.public_key).filter(|k| k.len() == 32).ok_or(ApiError::BadRequest("bad_key"))?;
        if namesproto::id_of(&public_key) != req.id {
            return Err(ApiError::BadRequest("id_is_not_the_hash_of_the_key"));
        }
        if !namesproto::valid_nonce(&req.nonce) {
            return Err(ApiError::BadRequest("bad_nonce"));
        }
        let signature = namesproto::unb64(&req.signature).filter(|s| s.len() == 64).ok_or(ApiError::BadRequest("bad_signature"))?;
        let ip = if req.ip.is_empty() { None } else { Some(namesproto::publishable_ip(&req.ip).ok_or(ApiError::Forbidden("ip_not_private"))?) };
        Names::check_time(req.time, now)?;
        if !namesproto::verify(&public_key, &namesproto::signing_input("register", &req.id, &req.ip, req.time, &req.nonce), &signature) {
            return Err(ApiError::Unauthorized("bad_signature"));
        }
        let mut inner = self.inner.lock().unwrap();
        Names::remember_nonce(&mut inner, &req.id, &req.nonce, now)?;
        let before = inner.records.get(&req.id).cloned();
        match &before {
            Some(r) => {
                if r.public_key != public_key {
                    return Err(ApiError::Conflict);
                }
                if now - r.last_request < self.limits.min_interval_secs {
                    return Err(ApiError::TooMany("too_often"));
                }
            }
            None => {
                if inner.records.len() >= self.limits.max_ids {
                    return Err(ApiError::TooMany("full"));
                }
            }
        }
        let txt = before.as_ref().map(|r| r.txt.clone()).unwrap_or_default();
        inner.records.insert(req.id.clone(), Record { public_key, ip, updated: now, last_request: now, txt });
        if let Err(e) = self.save(&inner) {
            // the answer must match what is on disk: put it back
            match before {
                Some(r) => inner.records.insert(req.id.clone(), r),
                None => inner.records.remove(&req.id),
            };
            return Err(ApiError::Internal(e));
        }
        Ok(())
    }

    pub fn acme(&self, req: &AcmeReq, now: i64) -> Result<(), ApiError> {
        if !namesproto::valid_id(&req.id) {
            return Err(ApiError::BadRequest("bad_id"));
        }
        let op = namesproto::acme_op(&req.op).ok_or(ApiError::BadRequest("bad_op"))?;
        if !namesproto::valid_txt_value(&req.value) {
            return Err(ApiError::BadRequest("bad_value"));
        }
        if !namesproto::valid_nonce(&req.nonce) {
            return Err(ApiError::BadRequest("bad_nonce"));
        }
        let signature = namesproto::unb64(&req.signature).filter(|s| s.len() == 64).ok_or(ApiError::BadRequest("bad_signature"))?;
        Names::check_time(req.time, now)?;
        let mut inner = self.inner.lock().unwrap();
        let public_key = inner.records.get(&req.id).map(|r| r.public_key.clone()).ok_or(ApiError::NotFound)?;
        if !namesproto::verify(&public_key, &namesproto::signing_input(op, &req.id, &req.value, req.time, &req.nonce), &signature) {
            return Err(ApiError::Unauthorized("bad_signature"));
        }
        Names::remember_nonce(&mut inner, &req.id, &req.nonce, now)?;
        let rec = inner.records.get_mut(&req.id).expect("checked above, under the same lock");
        rec.txt.retain(|(_, until)| *until > now);
        match req.op.as_str() {
            "set" => {
                rec.txt.retain(|(v, _)| v != &req.value);
                if rec.txt.len() >= MAX_TXT {
                    rec.txt.remove(0); // the oldest goes
                }
                rec.txt.push((req.value.clone(), now + TXT_TTL_SECS));
            }
            _ => rec.txt.retain(|(v, _)| v != &req.value),
        }
        Ok(())
    }

    /// Forgets records nobody refreshed for [`FORGET_AFTER_SECS`]. Returns how many.
    pub fn forget_old(&self, now: i64) -> usize {
        let mut inner = self.inner.lock().unwrap();
        let before = inner.records.len();
        inner.records.retain(|_, r| now - r.updated < FORGET_AFTER_SECS || r.ip.is_none());
        let gone = before - inner.records.len();
        if gone > 0 {
            let _ = self.save(&inner); // the next registration saves again; a failure here loses nothing new
        }
        gone
    }

    pub fn exists(&self, id: &str) -> bool {
        self.inner.lock().unwrap().records.contains_key(id)
    }

    pub fn address(&self, id: &str) -> Option<Ipv4Addr> {
        self.inner.lock().unwrap().records.get(id).and_then(|r| r.ip)
    }

    pub fn txt(&self, id: &str, now: i64) -> Vec<String> {
        self.inner.lock().unwrap().records.get(id).map(|r| r.txt.iter().filter(|(_, until)| *until > now).map(|(v, _)| v.clone()).collect()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::namesproto::Identity;

    fn me() -> (tempfile::TempDir, Identity) {
        let d = tempfile::tempdir().unwrap();
        let i = Identity::load_or_create(d.path()).unwrap();
        (d, i)
    }
    fn names() -> Names {
        Names::new(None, Limits::default())
    }
    const T: i64 = 1_800_000_000;

    #[test]
    fn registering_publishes_the_address_for_that_id_only() {
        let (_d, a) = me();
        let (_d2, b) = me();
        let n = names();
        n.register(&a.register("192.168.1.5", T), T).unwrap();
        assert_eq!(n.address(a.id()), Some(Ipv4Addr::new(192, 168, 1, 5)));
        assert_eq!(n.address(b.id()), None);
        assert!(!n.exists(b.id()));
    }

    #[test]
    fn a_public_address_is_refused_and_nothing_is_written() {
        let (_d, a) = me();
        let n = names();
        for ip in ["8.8.8.8", "127.0.0.1", "169.254.0.3", "fe80::1", "not an ip"] {
            assert_eq!(n.register(&a.register(ip, T), T), Err(ApiError::Forbidden("ip_not_private")), "{ip}");
        }
        assert_eq!(n.len(), 0);
    }

    #[test]
    fn the_signature_time_nonce_and_id_are_all_checked() {
        let (_d, a) = me();
        let (_d2, b) = me();
        let n = names();
        let ok = a.register("10.0.0.2", T);
        // someone else's signature on my ID
        let mut forged = a.register("10.0.0.2", T + 1);
        forged.signature = b.register("10.0.0.2", T + 1).signature;
        assert_eq!(n.register(&forged, T), Err(ApiError::Unauthorized("bad_signature")));
        // a changed address under the old signature
        let mut moved = ok.clone();
        moved.ip = "10.0.0.3".into();
        assert_eq!(n.register(&moved, T), Err(ApiError::Unauthorized("bad_signature")));
        // an ID that is not the hash of the key
        let mut liar = b.register("10.0.0.2", T);
        liar.id = a.id().to_string();
        assert_eq!(n.register(&liar, T), Err(ApiError::BadRequest("id_is_not_the_hash_of_the_key")));
        // too old and too new
        assert_eq!(n.register(&a.register("10.0.0.2", T - 301), T), Err(ApiError::Unauthorized("bad_time")));
        assert_eq!(n.register(&a.register("10.0.0.2", T + 301), T), Err(ApiError::Unauthorized("bad_time")));
        assert_eq!(n.len(), 0);
        // the real one works once; the same bytes again are a replay
        n.register(&ok, T).unwrap();
        assert_eq!(n.register(&ok, T + 10), Err(ApiError::Unauthorized("replayed")));
    }

    #[test]
    fn updates_are_limited_per_id_and_the_number_of_ids_is_limited() {
        let (_d, a) = me();
        let n = Names::new(None, Limits { max_ids: 1, min_interval_secs: 10 });
        n.register(&a.register("10.0.0.2", T), T).unwrap();
        assert_eq!(n.register(&a.register("10.0.0.2", T + 5), T + 5), Err(ApiError::TooMany("too_often")));
        n.register(&a.register("10.0.0.9", T + 10), T + 10).unwrap();
        assert_eq!(n.address(a.id()), Some(Ipv4Addr::new(10, 0, 0, 9)));
        let (_d2, b) = me();
        assert_eq!(n.register(&b.register("10.0.0.2", T), T), Err(ApiError::TooMany("full")));
    }

    fn value(c: char) -> String {
        c.to_string().repeat(43)
    }

    #[test]
    fn a_txt_value_needs_a_registered_id_a_proper_value_and_the_right_signature() {
        let (_d, a) = me();
        let (_d2, b) = me();
        let n = names();
        assert_eq!(n.acme(&a.acme("set", &value('A'), T), T), Err(ApiError::NotFound), "unknown ID: the bridge registers again");
        n.register(&a.register("10.0.0.2", T), T).unwrap();
        n.register(&b.register("10.0.0.3", T), T).unwrap();
        // b cannot write a's TXT
        let mut theirs = b.acme("set", &value('A'), T);
        theirs.id = a.id().to_string();
        assert_eq!(n.acme(&theirs, T), Err(ApiError::Unauthorized("bad_signature")));
        // only the 43-character form
        assert_eq!(n.acme(&a.acme("set", "short", T), T), Err(ApiError::BadRequest("bad_value")));
        assert_eq!(n.acme(&a.acme("set", &format!("{}!", "A".repeat(42)), T), T), Err(ApiError::BadRequest("bad_value")));
        let mut op = a.acme("set", &value('A'), T);
        op.op = "delete".into();
        assert_eq!(n.acme(&op, T), Err(ApiError::BadRequest("bad_op")));
        assert!(n.txt(a.id(), T).is_empty());
        n.acme(&a.acme("set", &value('A'), T), T).unwrap();
        assert_eq!(n.txt(a.id(), T), vec![value('A')]);
        assert!(n.txt(b.id(), T).is_empty());
    }

    #[test]
    fn txt_values_expire_are_capped_and_can_be_cleared() {
        let (_d, a) = me();
        let n = names();
        n.register(&a.register("10.0.0.2", T), T).unwrap();
        n.acme(&a.acme("set", &value('A'), T), T).unwrap();
        n.acme(&a.acme("set", &value('B'), T), T).unwrap();
        n.acme(&a.acme("set", &value('C'), T), T).unwrap();
        assert_eq!(n.txt(a.id(), T), vec![value('B'), value('C')], "at most 2: the oldest goes");
        n.acme(&a.acme("clear", &value('B'), T), T).unwrap();
        assert_eq!(n.txt(a.id(), T), vec![value('C')]);
        assert!(n.txt(a.id(), T + TXT_TTL_SECS).is_empty(), "a stale value does not linger");
    }

    #[test]
    fn records_survive_a_restart_and_a_damaged_record_is_left_out() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("names.json");
        let (_d, a) = me();
        {
            let n = Names::load(path.clone(), Limits::default(), T).unwrap().0;
            n.register(&a.register("192.168.7.7", T), T).unwrap();
        }
        let (n, problems) = Names::load(path.clone(), Limits::default(), T + 60).unwrap();
        assert!(problems.is_empty());
        assert_eq!(n.address(a.id()), Some(Ipv4Addr::new(192, 168, 7, 7)));
        // damage: a public address, written by hand
        let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        v["records"][a.id()]["ip"] = "8.8.8.8".into();
        std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
        let (n, problems) = Names::load(path, Limits::default(), T + 60).unwrap();
        assert_eq!(problems.len(), 1);
        assert!(!n.exists(a.id()));
    }

    #[test]
    fn a_failed_save_leaves_no_trace_of_the_registration() {
        let d = tempfile::tempdir().unwrap();
        // the path's parent is a file, so saving fails
        let blocker = d.path().join("blocker");
        std::fs::write(&blocker, b"x").unwrap();
        let n = Names::new(Some(blocker.join("names.json")), Limits::default());
        let (_d, a) = me();
        assert!(matches!(n.register(&a.register("10.0.0.2", T), T), Err(ApiError::Internal(_))));
        assert!(!n.exists(a.id()));
    }

    #[test]
    fn old_records_are_forgotten_but_the_services_own_identity_is_not() {
        let (_d, a) = me();
        let (_d2, svc) = me();
        let n = names();
        n.register(&a.register("10.0.0.2", T), T).unwrap();
        n.add_local(svc.id(), svc.public_key(), T);
        assert_eq!(n.forget_old(T + FORGET_AFTER_SECS + 1), 1);
        assert!(!n.exists(a.id()) && n.exists(svc.id()));
    }
}

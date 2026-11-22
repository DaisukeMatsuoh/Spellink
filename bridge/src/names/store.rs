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
/// Let's Encrypt counts new certificates per registered domain over 7 days.
pub const QUOTA_WINDOW_SECS: i64 = 7 * 86_400;
/// New IDs that may start ordering in a window; the other 5 of Let's Encrypt's 50 are kept for the
/// operator's own relays and name service (ADR 0017).
pub const DEFAULT_NEW_PER_WINDOW: usize = 45;
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
    /// The weekly allowance of new IDs ordering a certificate is used up (docs/spec/lan-cert.md L9):
    /// the unix time at which a place is free again.
    Quota(i64),
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
            ApiError::TooMany(_) | ApiError::Quota(_) => 429,
            ApiError::Internal(_) => 500,
        }
    }

    /// The short name sent to the caller. Internal details are not sent.
    pub fn name(&self) -> &str {
        match self {
            ApiError::BadRequest(s) | ApiError::Unauthorized(s) | ApiError::Forbidden(s) | ApiError::TooMany(s) => s,
            ApiError::NotFound => "unknown_id",
            ApiError::Conflict => "key_mismatch",
            ApiError::Quota(_) => "lan_quota",
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
    /// This process's own identity: never forgotten (everyone else is, when nobody refreshes it).
    local: bool,
    /// When this ID first asked for a challenge record, i.e. first started ordering a certificate
    /// (0: never). Only the first counts against the weekly allowance; renewals and retries do not.
    first_order: i64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedRecord {
    public_key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    ip: String,
    updated: i64,
    #[serde(default, skip_serializing_if = "is_zero")]
    first_order: i64,
}

fn is_zero(v: &i64) -> bool {
    *v == 0
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
    /// How many IDs may start ordering a certificate within [`QUOTA_WINDOW_SECS`].
    pub new_per_window: usize,
    /// IDs that are never counted nor refused (the operator's relays: the reserved places).
    pub infra: Vec<String>,
    /// The least time between two accepted requests for one ID.
    pub min_interval_secs: i64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_ids: 100_000, new_per_window: DEFAULT_NEW_PER_WINDOW, infra: Vec::new(), min_interval_secs: 2 }
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
                        (Some(public_key), Some(ip)) if namesproto::valid_id(&id) && now - r.updated < FORGET_AFTER_SECS && (0..=now + namesproto::MAX_SKEW_SECS).contains(&r.first_order) => {
                            inner.records.insert(id, Record { public_key, ip, updated: r.updated, last_request: 0, txt: Vec::new(), local: false, first_order: r.first_order });
                        }
                        (Some(_), Some(_)) if namesproto::valid_id(&id) && now - r.updated >= FORGET_AFTER_SECS => {} // old: forgotten on purpose
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
                .map(|(id, r)| (id.clone(), SavedRecord { public_key: namesproto::b64(&r.public_key), ip: r.ip.map(|i| i.to_string()).unwrap_or_default(), updated: r.updated, first_order: r.first_order }))
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

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Adds a record without a signature, for this process's own identity (it needs only the TXT
    /// record). Not saved: it is added again at every start.
    pub fn add_local(&self, id: &str, public_key: Vec<u8>, now: i64) {
        let mut inner = self.inner.lock().unwrap();
        inner.records.insert(id.to_string(), Record { public_key, ip: None, updated: now, last_request: 0, txt: Vec::new(), local: true, first_order: 0 });
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
        let before = inner.records.get(&req.id).cloned();
        // The checks that cost nothing to pass come before the nonce is remembered: a request that
        // is refused (full, too often, another key) must not use up the nonce memory.
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
        Names::remember_nonce(&mut inner, &req.id, &req.nonce, now)?;
        let local = before.as_ref().is_some_and(|r| r.local);
        let first_order = before.as_ref().map_or(0, |r| r.first_order);
        let txt = before.as_ref().map(|r| r.txt.clone()).unwrap_or_default();
        inner.records.insert(req.id.clone(), Record { public_key, ip, updated: now, last_request: now, txt, local, first_order });
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
        // Only an ID that has never ordered, and only when it asks to set a record, takes a place
        // from the weekly allowance. Like the other refusals, this comes before the nonce is
        // remembered. (L9)
        let starts_ordering = req.op == "set" && inner.records.get(&req.id).is_some_and(|r| r.first_order == 0 && !r.local) && !self.limits.infra.contains(&req.id);
        if starts_ordering {
            let since = now - QUOTA_WINDOW_SECS;
            let counted = inner.records.iter().filter(|(id, r)| r.first_order > since && !r.local && !self.limits.infra.contains(*id)).map(|(_, r)| r.first_order);
            let (n, oldest) = counted.fold((0usize, i64::MAX), |(n, oldest), t| (n + 1, oldest.min(t)));
            if n >= self.limits.new_per_window {
                return Err(ApiError::Quota(oldest.saturating_add(QUOTA_WINDOW_SECS)));
            }
        }
        Names::remember_nonce(&mut inner, &req.id, &req.nonce, now)?;
        let rec = inner.records.get_mut(&req.id).expect("checked above, under the same lock");
        // a signed request is a sign of life: an ID that still orders certificates is not forgotten
        let was = rec.updated;
        rec.updated = now;
        let took_a_place = starts_ordering;
        if took_a_place {
            rec.first_order = now;
        }
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
        if took_a_place || now - was > 86_400 {
            let _ = self.save(&inner); // the sign of life is kept across a restart; a failure loses nothing the next save does not fix
        }
        Ok(())
    }

    /// Forgets records nobody refreshed for [`FORGET_AFTER_SECS`]. Returns how many.
    pub fn forget_old(&self, now: i64) -> usize {
        let mut inner = self.inner.lock().unwrap();
        let before = inner.records.len();
        inner.records.retain(|_, r| r.local || now - r.updated < FORGET_AFTER_SECS);
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
    fn an_id_nobody_uses_is_forgotten_with_or_without_an_address_and_a_sign_of_life_keeps_it() {
        let n = names();
        let (_d1, with_ip) = me();
        let (_d2, no_ip) = me();
        let (_d3, orders) = me();
        let (_d4, own) = me();
        n.register(&with_ip.register("192.168.1.5", T), T).unwrap();
        n.register(&no_ip.register("", T), T).unwrap();
        n.register(&orders.register("", T), T).unwrap();
        n.add_local(own.id(), own.public_key(), T);
        // 100 days on: the one that orders a certificate shows it is alive
        let later = T + 100 * 86_400;
        n.acme(&orders.acme("set", &"A".repeat(43), later), later).unwrap();
        let much_later = T + FORGET_AFTER_SECS + 10;
        assert_eq!(n.forget_old(much_later), 2, "the one with an address and the one without go; no kind is exempt");
        assert!(!n.exists(with_ip.id()) && !n.exists(no_ip.id()));
        assert!(n.exists(orders.id()), "refreshed by a signed request");
        assert!(n.exists(own.id()), "this process's own identity is never forgotten");
    }

    #[test]
    fn a_refused_registration_does_not_use_up_the_nonce_memory() {
        let n = Names::new(None, Limits { max_ids: 1, min_interval_secs: 10, ..Limits::default() });
        let (_d1, a) = me();
        let (_d2, b) = me();
        n.register(&a.register("192.168.1.5", T), T).unwrap();
        let before = n.inner.lock().unwrap().nonces.len();
        // full: refused, and the nonce is not kept
        assert!(matches!(n.register(&b.register("192.168.1.6", T), T), Err(ApiError::TooMany("full"))));
        // too often: refused, and the nonce is not kept
        assert!(matches!(n.register(&a.register("192.168.1.7", T + 1), T + 1), Err(ApiError::TooMany("too_often"))));
        assert_eq!(n.inner.lock().unwrap().nonces.len(), before);
    }

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
        let n = Names::new(None, Limits { max_ids: 1, min_interval_secs: 10, ..Limits::default() });
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

    /// A name service that lets `n` new IDs start ordering a week; `infra` are never counted.
    fn with_allowance(n: usize, infra: Vec<String>) -> Names {
        Names::new(None, Limits { new_per_window: n, infra, ..Limits::default() })
    }

    #[test]
    fn only_a_new_id_starting_to_order_takes_a_place_and_a_full_week_refuses_it_with_the_time() {
        let ids: Vec<_> = (0..3).map(|_| me()).collect();
        let n = with_allowance(2, Vec::new());
        for (_, i) in &ids {
            n.register(&i.register("10.0.0.2", T), T).unwrap();
        }
        n.acme(&ids[0].1.acme("set", &value('A'), T + 10), T + 10).unwrap();
        n.acme(&ids[1].1.acme("set", &value('B'), T + 20), T + 20).unwrap();
        // the third: refused, with the time the oldest place is free again, and no nonce is kept
        let before = n.inner.lock().unwrap().nonces.len();
        assert_eq!(n.acme(&ids[2].1.acme("set", &value('C'), T + 30), T + 30), Err(ApiError::Quota(T + 10 + QUOTA_WINDOW_SECS)));
        assert_eq!(n.inner.lock().unwrap().nonces.len(), before);
        assert_eq!(ApiError::Quota(1).status(), 429);
        assert_eq!(ApiError::Quota(1).name(), "lan_quota");
        // clearing a record is never refused
        n.acme(&ids[2].1.acme("clear", &value('C'), T + 31), T + 31).unwrap();
        // the two that already ordered: renewals and retries are not counted and not refused
        n.acme(&ids[0].1.acme("set", &value('D'), T + 40), T + 40).unwrap();
        n.acme(&ids[1].1.acme("set", &value('E'), T + 50), T + 50).unwrap();
        // a place is free once the oldest is a week old
        let later = T + 10 + QUOTA_WINDOW_SECS + 1;
        n.acme(&ids[2].1.acme("set", &value('F'), later), later).unwrap();
    }

    #[test]
    fn the_operators_own_ids_are_neither_counted_nor_refused() {
        let (_d, relay) = me();
        let users: Vec<_> = (0..2).map(|_| me()).collect();
        let n = with_allowance(1, vec![relay.id().to_string()]);
        n.register(&relay.register("", T), T).unwrap();
        for (_, u) in &users {
            n.register(&u.register("10.0.0.2", T), T).unwrap();
        }
        // the relay orders while the week is full, and does not use up the user's place
        n.acme(&users[0].1.acme("set", &value('A'), T + 1), T + 1).unwrap();
        n.acme(&relay.acme("set", &value('R'), T + 2), T + 2).unwrap();
        assert!(matches!(n.acme(&users[1].1.acme("set", &value('B'), T + 3), T + 3), Err(ApiError::Quota(_))));
        // and this process's own identity too
        let (_d3, own) = me();
        n.add_local(own.id(), own.public_key(), T);
        n.acme(&own.acme("set", &value('L'), T + 4), T + 4).unwrap();
    }

    #[test]
    fn the_first_order_time_is_kept_across_a_new_registration_and_a_restart() {
        let dir = std::env::temp_dir().join(format!("spellink-quota-{}", std::process::id()));
        let path = dir.join("names.json");
        let (_d, a) = me();
        let n = Names::new(Some(path.clone()), Limits::default());
        n.register(&a.register("10.0.0.2", T), T).unwrap();
        n.acme(&a.acme("set", &value('A'), T + 5), T + 5).unwrap();
        n.register(&a.register("10.0.0.3", T + 20), T + 20).unwrap(); // the IP changed: same ID
        assert_eq!(n.inner.lock().unwrap().records[a.id()].first_order, T + 5);
        let (back, problems) = Names::load(path.clone(), Limits::default(), T + 30).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(back.inner.lock().unwrap().records[a.id()].first_order, T + 5);
        // a stored time in the far future is not believed: the record is left out
        let text = std::fs::read_to_string(&path).unwrap().replace(&format!("\"firstOrder\":{}", T + 5), &format!("\"firstOrder\":{}", T + 999_999));
        std::fs::write(&path, text).unwrap();
        let (bad, problems) = Names::load(path, Limits::default(), T + 30).unwrap();
        assert_eq!(bad.len(), 0);
        assert_eq!(problems.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }
}

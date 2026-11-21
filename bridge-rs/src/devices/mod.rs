//! The bridge's ledger of paired devices (ADR 0016, docs/spec/pairing.md; Go: internal/devices).
//!
//! Each device has a record. A record holds a first secret B (put into the device's own .snplg,
//! good only for pairing, with an expiry) and, once the owner has confirmed the pairing, a second
//! secret K that the device keeps and uses from then on; B is thrown away when K is known to have
//! arrived. The record's state machine is the table in docs/spec/pairing.md, generated into
//! `table.rs`.

use crate::timestamp::Timestamp;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

pub mod pkg;
pub mod registry;
pub mod table;

pub use registry::{Config, Conn, Hooks, Info, PackageData, Registry, Secret};

pub const LEDGER_VERSION: i64 = 1;
pub const SECRET_BYTES: usize = 32;
pub const ID_BYTES: usize = 8;
pub const MAX_NAME_CHARS: usize = 64;
pub const MAX_GEN: i64 = 1 << 30;
/// How many records the ledger holds.
pub const MAX_DEVICES: usize = 16;

/// What the owner (the command line, the pairing page) is told when an operation does not apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    UnknownDevice,
    NameTaken,
    BadName,
    Full,
    NoPending,
    BadCode,
    CodeRejected,
    NotEligible,
    /// The change took effect in memory but could not be written to the ledger (a revoked device
    /// must not be served, whatever the disk says).
    NotPersisted(String),
    AlreadyRevoked,
    Revoked,
    Expired,
    StillValid,
    BadRequest,
    /// Anything else: randomness, the disk, an id that cannot be found.
    Other(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::UnknownDevice => f.write_str("no such device"),
            Error::NameTaken => f.write_str("a device with this name already exists"),
            Error::BadName => f.write_str("a device name is 1 to 64 characters without control characters"),
            Error::Full => f.write_str("the ledger is full"),
            Error::NoPending => f.write_str("no device is showing a code"),
            Error::BadCode => f.write_str("a pairing code is exactly eight digits"),
            Error::CodeRejected => f.write_str("the code was not accepted"),
            Error::NotEligible => f.write_str("this device cannot connect with that secret in its current state"),
            Error::NotPersisted(e) => write!(f, "the change could not be written to the ledger: {e}"),
            Error::AlreadyRevoked => f.write_str("the device is already revoked"),
            Error::Revoked => f.write_str("the device is revoked; add a new one"),
            Error::Expired => f.write_str("the first secret has expired; reissue the device"),
            Error::StillValid => f.write_str("revoke the device before forgetting it"),
            Error::BadRequest => f.write_str("malformed pairing request"),
            Error::Other(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for Error {}

fn is_zero_time(t: &Timestamp) -> bool {
    t.is_zero()
}
fn zero_time() -> Timestamp {
    Timestamp::ZERO
}
fn empty(s: &String) -> bool {
    s.is_empty()
}

/// One device in the ledger as it is stored. Requests waiting for the owner's confirmation
/// (state Requested) are never stored: after a restart the record is simply issued again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "gen")]
    pub generation: i64,
    /// issued, granted, active, expired or revoked (see `state_code`)
    #[serde(default)]
    pub state: String,
    #[serde(default, skip_serializing_if = "empty")]
    pub b: String,
    #[serde(default, skip_serializing_if = "empty")]
    pub k: String,
    #[serde(default = "zero_time")]
    pub issued_at: Timestamp,
    #[serde(default = "zero_time", skip_serializing_if = "is_zero_time")]
    pub revoked_at: Timestamp,
    #[serde(default = "zero_time", skip_serializing_if = "is_zero_time")]
    pub last_seen: Timestamp,
}

impl Default for Record {
    fn default() -> Self {
        Record { id: String::new(), name: String::new(), generation: 0, state: String::new(), b: String::new(), k: String::new(), issued_at: Timestamp::ZERO, revoked_at: Timestamp::ZERO, last_seen: Timestamp::ZERO }
    }
}

#[derive(Serialize, Deserialize)]
struct LedgerFile {
    #[serde(default)]
    version: i64,
    #[serde(default)]
    devices: Vec<Record>,
}

// The names a record's state is stored under. They are the ledger's file format, so they do not
// follow the names in the code (Issued, …): a rename in the code must not change the file. A
// record that is waiting for the owner's code (Requested) is stored as issued.
pub const STATE_ISSUED: &str = "issued";
pub const STATE_GRANTED: &str = "granted";
pub const STATE_ACTIVE: &str = "active";
pub const STATE_EXPIRED: &str = "expired";
pub const STATE_REVOKED: &str = "revoked";

use table::DevState;

pub fn state_code(s: DevState) -> &'static str {
    match s {
        DevState::Issued | DevState::Requested => STATE_ISSUED,
        DevState::Granted => STATE_GRANTED,
        DevState::Active => STATE_ACTIVE,
        DevState::Expired => STATE_EXPIRED,
        DevState::Revoked => STATE_REVOKED,
        // a record that does not exist is never written
        DevState::NoRecord => "",
    }
}

pub fn code_state(code: &str) -> Option<DevState> {
    Some(match code {
        STATE_ISSUED => DevState::Issued,
        STATE_GRANTED => DevState::Granted,
        STATE_ACTIVE => DevState::Active,
        STATE_EXPIRED => DevState::Expired,
        STATE_REVOKED => DevState::Revoked,
        _ => return None,
    })
}

/// Whether `s` is a usable device name: 1 to 64 characters, none of them control.
pub fn valid_name(s: &str) -> bool {
    let n = s.chars().count();
    (1..=MAX_NAME_CHARS).contains(&n) && !s.chars().any(|c| c.is_control())
}

/// Whether `s` is a 32-byte secret in unpadded base64url (and the canonical text of it).
pub fn valid_secret(s: &str) -> bool {
    match URL_SAFE_NO_PAD.decode(s) {
        Ok(b) => b.len() == SECRET_BYTES && URL_SAFE_NO_PAD.encode(b) == s,
        Err(_) => false,
    }
}

/// Whether `s` is a pairing code: exactly eight (ASCII) digits.
pub fn valid_code(s: &str) -> bool {
    s.len() == 8 && s.bytes().all(|b| b.is_ascii_digit())
}

/// Whether `s` is a device id: sixteen lowercase hex digits.
pub fn valid_id(s: &str) -> bool {
    s.len() == 16 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Validates one stored record. Returns the record to use (B is dropped from an active record,
/// ADR 0016) or an error that says what is wrong.
fn check(mut r: Record) -> Result<Record, String> {
    if !valid_id(&r.id) {
        return Err("bad id".into());
    }
    if !valid_name(&r.name) {
        return Err("bad name".into());
    }
    if r.generation < 1 || r.generation > MAX_GEN {
        return Err(format!("bad gen {}", r.generation));
    }
    if !r.b.is_empty() && !valid_secret(&r.b) {
        return Err("bad B".into());
    }
    if !r.k.is_empty() && !valid_secret(&r.k) {
        return Err("bad K".into());
    }
    let (has_b, has_k) = (!r.b.is_empty(), !r.k.is_empty());
    let ok = match r.state.as_str() {
        STATE_ISSUED => has_b && !has_k,
        STATE_GRANTED => has_b && has_k,
        STATE_ACTIVE => {
            r.b.clear(); // a B left over from an interrupted switch is dropped
            has_k
        }
        STATE_EXPIRED | STATE_REVOKED => !has_b && !has_k,
        other => return Err(format!("bad state {other:?}")),
    };
    if !ok {
        return Err(format!("state {} does not fit the secrets it holds (B {has_b}, K {has_k})", r.state));
    }
    Ok(r)
}

/// Reads a ledger. A file that is not a ledger at all is an error (the bridge does not start with
/// a ledger it cannot read: that would serve nobody, silently). Single records that are not usable
/// are left out and reported, so one bad record cannot hide the rest.
pub fn decode(data: &[u8]) -> Result<(Vec<Record>, Vec<String>), String> {
    // like Go's decoder: the first JSON value counts, whatever follows it
    let mut stream = serde_json::Deserializer::from_slice(data).into_iter::<LedgerFile>();
    let f = match stream.next() {
        Some(Ok(f)) => f,
        Some(Err(e)) => return Err(format!("not a device ledger: {e}")),
        None => return Err("not a device ledger: empty".into()),
    };
    if f.version != LEDGER_VERSION {
        return Err(format!("ledger version {} is not supported (this bridge reads {LEDGER_VERSION})", f.version));
    }
    let (mut ids, mut names) = (std::collections::HashSet::new(), std::collections::HashSet::new());
    let (mut recs, mut skipped) = (Vec::new(), Vec::new());
    for (i, r) in f.devices.into_iter().enumerate() {
        match check(r) {
            Err(e) => skipped.push(format!("record {i}: {e}")),
            Ok(r) if ids.contains(&r.id) => skipped.push(format!("record {i}: duplicate id")),
            Ok(r) if names.contains(&r.name) => skipped.push(format!("record {i}: duplicate name")),
            Ok(_) if recs.len() >= MAX_DEVICES => skipped.push(format!("record {i}: more than {MAX_DEVICES} devices")),
            Ok(r) => {
                ids.insert(r.id.clone());
                names.insert(r.name.clone());
                recs.push(r);
            }
        }
    }
    Ok((recs, skipped))
}

pub fn encode(recs: &[Record]) -> Result<Vec<u8>, String> {
    #[derive(Serialize)]
    struct Out<'a> {
        version: i64,
        devices: &'a [Record],
    }
    serde_json::to_vec_pretty(&Out { version: LEDGER_VERSION, devices: recs }).map_err(|e| e.to_string())
}

/// Keeps the ledger's bytes somewhere.
pub trait Store: Send + Sync {
    /// The stored bytes; a ledger that was never written is `None`.
    fn load(&self) -> Result<Option<Vec<u8>>, String>;
    /// Replaces the stored bytes. Returns only when the new bytes can be read back.
    fn save(&self, data: &[u8]) -> Result<(), String>;
}

type ReadFn = Arc<dyn Fn(&std::path::Path) -> std::io::Result<Vec<u8>> + Send + Sync>;

/// Keeps the ledger in one file (mode 0600): written to a temporary file next to it and renamed
/// over it, so a power cut leaves the old or the new ledger, and read back to check.
pub struct FileStore {
    pub path: PathBuf,
    /// Replaces `std::fs::read` (a test does).
    pub read: Option<ReadFn>,
}

impl FileStore {
    pub fn new(path: impl Into<PathBuf>) -> FileStore {
        FileStore { path: path.into(), read: None }
    }
}

impl Store for FileStore {
    fn load(&self) -> Result<Option<Vec<u8>>, String> {
        match std::fs::read(&self.path) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", self.path.display())),
        }
    }

    fn save(&self, data: &[u8]) -> Result<(), String> {
        let shown = self.path.display();
        let dir = self.path.parent().filter(|d| !d.as_os_str().is_empty()).map(|d| d.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
        crate::util::create_private_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let tmp = dir.join(format!(".devices-{:016x}.tmp", rand::random::<u64>()));
        let result = (|| -> std::io::Result<()> {
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
            let mut f = opts.open(&tmp)?;
            f.write_all(data)?;
            f.sync_all()?;
            drop(f);
            std::fs::rename(&tmp, &self.path)
        })();
        if let Err(e) = result {
            let _ = std::fs::remove_file(&tmp); // after a successful rename the name is gone and this does nothing
            return Err(format!("{shown}: {e}"));
        }
        let back = match &self.read {
            Some(r) => r(&self.path),
            None => std::fs::read(&self.path),
        }
        .map_err(|e| format!("{shown}: {e}"))?;
        if back != data {
            return Err("ledger read back differs from what was written".into());
        }
        Ok(())
    }
}

/// A fresh random source: fills the buffer or says why not.
pub type Rand = Box<dyn FnMut(&mut [u8]) -> Result<(), String> + Send>;

pub fn os_rand() -> Rand {
    Box::new(|b: &mut [u8]| {
        use rand::RngCore;
        rand::rng().fill_bytes(b); // the operating system's generator: it does not fail here
        Ok(())
    })
}

/// A fresh device id from the random source.
pub fn new_id(rnd: &mut Rand) -> Result<String, String> {
    let mut b = [0u8; ID_BYTES];
    rnd(&mut b)?;
    Ok(hex::encode(b))
}

/// A fresh 32-byte secret in unpadded base64url.
pub fn new_secret(rnd: &mut Rand) -> Result<String, String> {
    let mut b = [0u8; SECRET_BYTES];
    rnd(&mut b)?;
    Ok(URL_SAFE_NO_PAD.encode(b))
}

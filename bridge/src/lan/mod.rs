//! Direct wss:// at home (ADR 0017, docs/spec/lan-cert.md): the bridge registers its name with the
//! name service and gets its own certificate from Let's Encrypt, then serves on the LAN with it.
//!
//! Two state machines run here, one for the registration and one for the certificate. Their
//! tables are generated from the document (`reg_table.rs`, `cert_table.rs`); each runs as one task
//! that receives events, so a state only ever changes in one place. The results of attempts carry
//! the attempt's number, and a result of an older attempt is dropped (L4), never guessed at.

pub mod cert_table;
pub mod client;
pub mod reg_table;

use crate::acme::{BoxFut, CertFiles, Checked, Issued, OrderError, Orderer, Publisher};
use crate::tlsserve::CertResolver;
use cert_table::{CertAction, CertEvent, CertState};
use reg_table::{RegAction, RegEvent, RegState};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

pub const DEFAULT_NAMES: &str = "https://names.spellink.uk:8443";
pub const DEFAULT_ZONE: &str = "lan.spellink.uk";

/// The user's setting (config.json "lan").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    /// Off unless set: the name service must be running before this is worth turning on.
    #[serde(default)]
    pub enabled: bool,
    /// The name service (HTTPS).
    #[serde(default = "default_names")]
    pub names: String,
    /// The zone the name service answers for; the bridge's name is `<id>.<zone>`.
    #[serde(default = "default_zone")]
    pub zone: String,
    /// Optional: where Let's Encrypt writes about expiry.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub email: String,
    /// "staging" gets certificates browsers do not trust, for trying without the real limits.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ca: String,
    /// This bridge's name, `<id>.<zone>`: set when the bridge starts, never saved.
    #[serde(skip)]
    pub host: String,
    /// Old settings (the Cloudflare-token way, ADR 0011) are kept so that the file is not damaged,
    /// but they are not used.
    #[serde(flatten)]
    pub other: std::collections::BTreeMap<String, serde_json::Value>,
}

fn default_names() -> String {
    DEFAULT_NAMES.into()
}
fn default_zone() -> String {
    DEFAULT_ZONE.into()
}

impl Default for Config {
    fn default() -> Self {
        Config { enabled: false, names: default_names(), zone: default_zone(), email: String::new(), ca: String::new(), host: String::new(), other: Default::default() }
    }
}

impl Config {
    /// What is wrong with the setting, or None when usable.
    pub fn valid(&self) -> Option<String> {
        if let Err(e) = client::names_url(&self.names) {
            return Some(format!("lan.names: {e}"));
        }
        if crate::names::dns::labels_of(&self.zone).is_none_or(|l| l.len() < 2) {
            return Some(format!("lan.zone {:?} is not a DNS name", self.zone));
        }
        match self.ca.as_str() {
            "" | "staging" => {}
            other => return Some(format!("lan.ca {other:?} is not \"\" or \"staging\"")),
        }
        if self.email.len() > 254 || self.email.contains(char::is_whitespace) {
            return Some("lan.email is not an address".into());
        }
        None
    }

    pub fn directory(&self) -> &'static str {
        if self.ca == "staging" { crate::acme::Acme::staging() } else { crate::acme::Acme::production() }
    }

    /// The setting's deprecated keys that are still in the file.
    pub fn unused_keys(&self) -> Vec<&str> {
        self.other.keys().map(String::as_str).collect()
    }
}

/// Registers this machine's name with the name service.
pub trait Registrar: Send + Sync + 'static {
    /// `ip` is the private IPv4 address to publish, or "" for a service that only needs the TXT.
    fn register<'a>(&'a self, ip: &'a str) -> BoxFut<'a, Result<(), String>>;
}

/// Where this machine's address comes from.
pub trait Addresses: Send + Sync + 'static {
    /// The address to publish now: Some("") for a service that publishes none, None when the
    /// machine has no usable address.
    fn current(&self) -> Option<String>;
}

/// The private IPv4 address of this machine on its LAN (the same on every call while the network
/// does not change).
pub struct LanAddress;

impl Addresses for LanAddress {
    fn current(&self) -> Option<String> {
        let mut found: Vec<std::net::Ipv4Addr> = if_addrs::get_if_addrs()
            .ok()?
            .into_iter()
            .filter(|i| !i.is_loopback())
            .filter_map(|i| match i.ip() {
                std::net::IpAddr::V4(v4) if crate::namesproto::publishable_ip(&v4.to_string()).is_some() => Some(v4),
                _ => None,
            })
            .collect();
        found.sort(); // stable between calls: a change of the first one is a real change
        // The address the machine itself uses to reach the outside is the one on the LAN; a docker
        // bridge (172.17.x) or a VPN (10.x) sorts first just as well and is not reachable from the
        // device. Connecting a UDP socket sends nothing: it only makes the system pick a route.
        let routed = std::net::UdpSocket::bind(("0.0.0.0", 0)).and_then(|s| s.connect(("192.0.2.1", 9)).and_then(|_| s.local_addr())).ok().and_then(|a| match a.ip() {
            std::net::IpAddr::V4(v4) if found.contains(&v4) => Some(v4),
            _ => None,
        });
        routed.or_else(|| found.first().copied()).map(|ip| ip.to_string())
    }
}

/// How the first retry after a failure waits, in seconds, and how long at most (docs/spec/lan-cert.md
/// L6). The CA's own time wins when it gives one.
pub fn backoff(failures: u32) -> i64 {
    match failures {
        0 | 1 => 60,
        2 => 300,
        3 => 1800,
        4 => 7200,
        _ => 21_600,
    }
}

/// Renewal starts when 30 days are left, or a third of the life, whichever is less.
pub fn renew_at(c: &Checked) -> i64 {
    c.not_after - (30 * 86_400).min((c.not_after - c.not_before) / 3)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub reg: RegState,
    pub cert: CertState,
    pub not_after: Option<i64>,
    pub error: String,
}

impl Status {
    /// One line for the settings screen and the log.
    pub fn describe(&self) -> String {
        let serving = matches!(self.cert, CertState::Valid | CertState::Renewing | CertState::RenewWait);
        match (self.cert, serving) {
            (CertState::Off, _) => "off".into(),
            (_, true) => "ready".into(),
            _ if self.error.is_empty() => "preparing (the relay works meanwhile)".into(),
            _ => format!("preparing (the relay works meanwhile): {}", self.error),
        }
    }
    pub fn serving(&self) -> bool {
        matches!(self.cert, CertState::Valid | CertState::Renewing | CertState::RenewWait)
    }
}

pub struct Timing {
    /// How often deadlines and the machine's address are looked at (a deadline is a wall-clock
    /// time, so a sleeping machine does not make a timer late by the length of its sleep).
    pub tick: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing { tick: Duration::from_secs(30) }
    }
}

pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;
pub type Log = Arc<dyn Fn(&str) + Send + Sync>;

pub fn system_clock() -> Clock {
    Arc::new(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0))
}

pub struct Parts {
    /// The name the certificate is for, e.g. `<id>.lan.spellink.uk`.
    pub domain: String,
    pub registrar: Arc<dyn Registrar>,
    pub addresses: Arc<dyn Addresses>,
    pub publisher: Arc<dyn Publisher>,
    pub orderer: Arc<dyn Orderer>,
    pub files: CertFiles,
    pub resolver: Arc<CertResolver>,
    pub clock: Clock,
    pub log: Log,
    pub timing: Timing,
}

enum RegMsg {
    Event(RegEvent),
    Result { attempt: u64, ip: Option<String>, result: Result<(), String> },
}

enum CertMsg {
    Event(CertEvent),
    Result { attempt: u64, result: Result<Issued, OrderError> },
}

pub struct Lan {
    reg: mpsc::UnboundedSender<RegMsg>,
    cert: mpsc::UnboundedSender<CertMsg>,
    status: watch::Receiver<Status>,
    pub resolver: Arc<CertResolver>,
}

impl Lan {
    pub fn status(&self) -> Status {
        self.status.borrow().clone()
    }

    pub fn watch(&self) -> watch::Receiver<Status> {
        self.status.clone()
    }

    /// Stops (the configuration was turned off, or the bridge stops).
    pub fn disable(&self) {
        let _ = self.cert.send(CertMsg::Event(CertEvent::Disable));
        let _ = self.reg.send(RegMsg::Event(RegEvent::Disable));
    }

    /// Starts both machines. The certificate machine is enabled first, so that it is waiting
    /// when the registration succeeds.
    pub fn start(parts: Parts, cancel: CancellationToken) -> Lan {
        let (reg_tx, reg_rx) = mpsc::unbounded_channel();
        let (cert_tx, cert_rx) = mpsc::unbounded_channel();
        let (status_tx, status_rx) = watch::channel(Status { reg: RegState::Off, cert: CertState::Off, not_after: None, error: String::new() });
        let status_tx = Arc::new(status_tx);
        let resolver = parts.resolver.clone();
        let shared = Arc::new(Shared { domain: parts.domain.clone(), clock: parts.clock.clone(), log: parts.log.clone(), status: status_tx });
        let cert = CertActor {
            shared: shared.clone(),
            state: CertState::Off,
            orderer: parts.orderer,
            publisher: parts.publisher,
            files: parts.files,
            resolver: parts.resolver,
            tx: cert_tx.clone(),
            on_lost: reg_tx.clone(),
            attempt: 0,
            failures: 0,
            retry: None,
            renew: None,
            expires: None,
            not_after: None,
            pending: None,
            rate_limited_until: None,
            error: String::new(),
        };
        let reg = RegActor {
            shared,
            state: RegState::Off,
            registrar: parts.registrar,
            addresses: parts.addresses,
            tx: reg_tx.clone(),
            on_registered: cert_tx.clone(),
            attempt: 0,
            failures: 0,
            retry: None,
            attempted_ip: None,
            error: String::new(),
        };
        tokio::spawn(cert.run(cert_rx, parts.timing.tick, cancel.clone()));
        tokio::spawn(reg.run(reg_rx, parts.timing.tick, cancel));
        let _ = cert_tx.send(CertMsg::Event(CertEvent::Enable)); // turned into EnableWithCert when a certificate is saved
        let _ = reg_tx.send(RegMsg::Event(RegEvent::Enable));
        Lan { reg: reg_tx, cert: cert_tx, status: status_rx, resolver }
    }
}

struct Shared {
    domain: String,
    clock: Clock,
    log: Log,
    status: Arc<watch::Sender<Status>>,
}

impl Shared {
    fn now(&self) -> i64 {
        (self.clock)()
    }
    fn log(&self, s: &str) {
        (self.log)(&format!("lan: {s}"));
    }
    fn publish(&self, f: impl FnOnce(&mut Status)) {
        self.status.send_modify(f);
    }
}

// ---------------------------------------------------------------- registration

struct RegActor {
    shared: Arc<Shared>,
    state: RegState,
    registrar: Arc<dyn Registrar>,
    addresses: Arc<dyn Addresses>,
    tx: mpsc::UnboundedSender<RegMsg>,
    on_registered: mpsc::UnboundedSender<CertMsg>,
    attempt: u64,
    failures: u32,
    retry: Option<i64>,
    /// The address the latest attempt used (None: there was none). A change of the machine's
    /// address compared with this is `AddressChanges`.
    attempted_ip: Option<String>,
    error: String,
}

impl RegActor {
    async fn run(mut self, mut rx: mpsc::UnboundedReceiver<RegMsg>, tick: Duration, cancel: CancellationToken) {
        let mut ticker = tokio::time::interval(tick);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = ticker.tick() => self.on_tick(),
                msg = rx.recv() => match msg {
                    None => return,
                    Some(RegMsg::Event(e)) => self.step(e),
                    Some(RegMsg::Result { attempt, ip, result }) => {
                        if attempt != self.attempt {
                            continue; // the result of an attempt that was given up (L4)
                        }
                        match result {
                            Ok(()) => { self.attempted_ip = ip; self.step(RegEvent::RegisterOk) }
                            Err(e) => { self.error = e; self.step(RegEvent::RegisterFails) }
                        }
                    }
                },
            }
        }
    }

    fn on_tick(&mut self) {
        let now = self.shared.now();
        if self.retry.is_some_and(|t| now >= t) {
            self.retry = None;
            self.step(RegEvent::RetryDue);
        }
        if self.state != RegState::Off && self.addresses.current() != self.attempted_ip {
            self.step(RegEvent::AddressChanges);
        }
    }

    fn step(&mut self, ev: RegEvent) {
        use reg_table::Cell;
        let from = self.state;
        match reg_table::cell(from, ev) {
            Cell::Never(note) => self.shared.log(&format!("registration: {} cannot happen in {} (note {note}); ignored", ev_name(&format!("{ev:?}")), from.name())),
            Cell::Ignore(_) => {}
            Cell::IgnoreDo(a) => self.act(a),
            Cell::Do { action, next } => {
                self.act(action);
                self.state = next;
            }
            Cell::Guard { .. } => self.shared.log("registration: a guarded cell, which this table does not have"),
        }
        // a retry timer exists only while waiting
        if self.state != RegState::RegisterWait {
            self.retry = None;
        }
        let (state, error) = (self.state, self.error.clone());
        self.shared.publish(|s| {
            s.reg = state;
            if state == RegState::Registered {
                s.error.clear();
            } else if state == RegState::RegisterWait {
                s.error = format!("registering: {error}");
            }
        });
    }

    fn start_attempt(&mut self) {
        self.attempt += 1;
        self.retry = None;
        let attempt = self.attempt;
        let ip = self.addresses.current();
        self.attempted_ip = ip.clone();
        let (tx, registrar) = (self.tx.clone(), self.registrar.clone());
        tokio::spawn(async move {
            let result = match &ip {
                None => Err("no private IPv4 address on this machine".to_string()),
                Some(ip) => registrar.register(ip).await,
            };
            let _ = tx.send(RegMsg::Result { attempt, ip, result });
        });
    }

    fn act(&mut self, a: RegAction) {
        match a {
            RegAction::N1 => self.start_attempt(),
            RegAction::N2 => {
                self.attempt += 1;
                self.retry = None;
                self.attempted_ip = None;
            }
            RegAction::N3 => {
                self.failures += 1;
                let wait = backoff(self.failures);
                self.retry = Some(self.shared.now() + wait);
                self.shared.log(&format!("registration failed ({}); trying again in {wait} s", self.error));
            }
            RegAction::N4 => {
                self.failures = 0;
                self.error.clear();
                self.shared.log(&format!("registered {}", self.shared.domain));
                let _ = self.on_registered.send(CertMsg::Event(CertEvent::RegistrationReady));
            }
            RegAction::N5 => self.start_attempt(),
        }
    }
}

fn ev_name(debug: &str) -> String {
    debug.to_string()
}

// ---------------------------------------------------------------- certificate

struct CertActor {
    shared: Arc<Shared>,
    state: CertState,
    orderer: Arc<dyn Orderer>,
    publisher: Arc<dyn Publisher>,
    files: CertFiles,
    resolver: Arc<CertResolver>,
    tx: mpsc::UnboundedSender<CertMsg>,
    on_lost: mpsc::UnboundedSender<RegMsg>,
    attempt: u64,
    failures: u32,
    retry: Option<i64>,
    renew: Option<i64>,
    expires: Option<i64>,
    not_after: Option<i64>,
    /// The certificate an `Issued` / `EnableWithCert` event brings (already checked and saved).
    pending: Option<Checked>,
    rate_limited_until: Option<i64>,
    error: String,
}

impl CertActor {
    async fn run(mut self, mut rx: mpsc::UnboundedReceiver<CertMsg>, tick: Duration, cancel: CancellationToken) {
        let mut ticker = tokio::time::interval(tick);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = ticker.tick() => self.on_tick(),
                msg = rx.recv() => match msg {
                    None => return,
                    Some(CertMsg::Event(CertEvent::Enable)) => self.enable(),
                    Some(CertMsg::Event(e)) => self.step(e),
                    Some(CertMsg::Result { attempt, result }) => {
                        if attempt != self.attempt {
                            continue; // an order that was given up (L4)
                        }
                        self.on_result(result);
                    }
                },
            }
        }
    }

    /// `Enable`, or `EnableWithCert` when a certificate that is valid now was kept.
    fn enable(&mut self) {
        let now = self.shared.now();
        if let Some((chain, key, checked)) = self.files.load(&self.shared.domain, now) {
            match self.resolver.set(&chain, &key) {
                Ok(()) => {
                    self.pending = Some(checked);
                    return self.step(CertEvent::EnableWithCert);
                }
                Err(e) => self.shared.log(&format!("the saved certificate cannot be used ({e}); getting a new one")),
            }
        }
        self.step(CertEvent::Enable);
    }

    fn on_tick(&mut self) {
        let now = self.shared.now();
        // in this order: a retry, a renewal, then the end of the certificate. A step can make a
        // later one not apply (the deadline is dropped with the state), so each is checked again.
        if self.retry.is_some_and(|t| now >= t) {
            self.retry = None;
            self.step(CertEvent::RetryDue);
        }
        if self.renew.is_some_and(|t| now >= t) {
            self.renew = None;
            self.step(CertEvent::RenewDue);
        }
        if self.expires.is_some_and(|t| now >= t) {
            self.expires = None;
            self.step(CertEvent::CertExpires);
        }
    }

    /// An order finished. A certificate is checked and saved before it counts (I33, L7); one that
    /// is not good is a failed order.
    fn on_result(&mut self, result: Result<Issued, OrderError>) {
        match result {
            Ok(issued) => {
                let now = self.shared.now();
                match self.files.save(&issued.chain_pem, &issued.key_pem, &self.shared.domain, now).and_then(|c| self.resolver.set(&issued.chain_pem, &issued.key_pem).map(|()| c)) {
                    Ok(checked) => {
                        self.pending = Some(checked);
                        self.step(CertEvent::Issued);
                    }
                    Err(e) => {
                        self.error = format!("the certificate that was issued is not usable: {e}");
                        self.step(CertEvent::OrderFails);
                    }
                }
            }
            Err(OrderError::RateLimited(until)) => {
                self.error = "Let's Encrypt asked to wait (too many certificates for now)".into();
                self.rate_limited_until = until;
                self.step(CertEvent::RateLimited);
            }
            Err(OrderError::RegistrationLost) => {
                self.error = "the name service does not know this bridge; registering again".into();
                let _ = self.on_lost.send(RegMsg::Event(RegEvent::RegistrationLost));
                self.step(CertEvent::OrderFails);
            }
            Err(OrderError::Failed(e)) => {
                self.error = e;
                self.step(CertEvent::OrderFails);
            }
        }
    }

    fn step(&mut self, ev: CertEvent) {
        use cert_table::Cell;
        let from = self.state;
        match cert_table::cell(from, ev) {
            Cell::Never(note) => self.shared.log(&format!("certificate: {ev:?} cannot happen in {} (note {note}); ignored", from.name())),
            Cell::Ignore(_) => {}
            Cell::IgnoreDo(a) => self.act(a),
            Cell::Do { action, next } => {
                self.act(action);
                self.state = next;
            }
            Cell::Guard { .. } => self.shared.log("certificate: a guarded cell, which this table does not have"),
        }
        // a deadline exists only in the states where its event applies (the table's notes d)
        use CertState::*;
        if !matches!(self.state, OrderWait | RenewWait) {
            self.retry = None;
        }
        if self.state != Valid {
            self.renew = None;
        }
        if !matches!(self.state, Valid | Renewing | RenewWait) {
            self.expires = None;
        }
        if matches!(self.state, Off | AwaitRegistration | Ordering | OrderWait) {
            self.not_after = None;
        }
        let (state, not_after, error) = (self.state, self.not_after, self.error.clone());
        self.shared.publish(|s| {
            s.cert = state;
            s.not_after = not_after;
            s.error = if matches!(state, Valid | Renewing) { String::new() } else { error };
        });
    }

    fn start_order(&mut self) {
        self.attempt += 1;
        self.retry = None;
        self.renew = None;
        let attempt = self.attempt;
        let (tx, orderer, publisher, domain) = (self.tx.clone(), self.orderer.clone(), self.publisher.clone(), self.shared.domain.clone());
        self.shared.log(&format!("ordering a certificate for {domain}"));
        tokio::spawn(async move {
            let result = orderer.order(&domain, publisher.as_ref()).await;
            let _ = tx.send(CertMsg::Result { attempt, result });
        });
    }

    fn keep_deadlines(&mut self) {
        if let Some(c) = self.pending.take() {
            self.renew = Some(renew_at(&c));
            self.expires = Some(c.not_after);
            self.not_after = Some(c.not_after);
        }
    }

    fn act(&mut self, a: CertAction) {
        match a {
            CertAction::C0 => self.shared.log("waiting for the registration"),
            CertAction::C1 => self.start_order(),
            CertAction::C2 => {
                self.attempt += 1;
                self.retry = None;
                self.renew = None;
                self.expires = None;
                self.resolver.clear();
            }
            CertAction::C3 => {
                self.failures = 0;
                self.error.clear();
                self.keep_deadlines();
                self.shared.log("the certificate is in place");
            }
            CertAction::C4 => {
                self.failures += 1;
                let now = self.shared.now();
                let wait_until = match self.rate_limited_until.take() {
                    Some(t) => t.max(now + 60),
                    None => now + backoff(self.failures),
                };
                self.retry = Some(wait_until);
                self.shared.log(&format!("no certificate yet ({}); trying again in {} s", self.error, wait_until - now));
            }
            CertAction::C5 => {
                self.failures = 0;
                self.keep_deadlines();
                self.shared.log("serving the saved certificate");
            }
            CertAction::C6 => {
                self.resolver.clear();
                self.not_after = None;
                self.shared.log("the certificate has expired; not serving it");
            }
        }
    }
}

#[cfg(test)]
mod tests;

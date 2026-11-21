//! The two machines of docs/spec/lan-cert.md, driven through their real task with a scripted CA,
//! a scripted name service and a clock the test moves.

use super::*;
use crate::acme::PublishError;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

const T0: i64 = 1_800_000_000;
const DOMAIN: &str = "0123456789abcdef0123456789abcdef.lan.test";

fn cert(domain: &str, not_before: i64, not_after: i64) -> Issued {
    let key = rcgen::KeyPair::generate().unwrap();
    let mut p = rcgen::CertificateParams::new(vec![domain.to_string()]).unwrap();
    let t = |s: i64| time::OffsetDateTime::from_unix_timestamp(s).unwrap();
    p.not_before = t(not_before);
    p.not_after = t(not_after);
    Issued { chain_pem: p.self_signed(&key).unwrap().pem(), key_pem: key.serialize_pem() }
}

#[derive(Default)]
struct Script<T> {
    queue: Mutex<VecDeque<T>>,
}

struct World {
    now: Arc<AtomicI64>,
    orders: Arc<AtomicUsize>,
    registers: Arc<Mutex<Vec<String>>>,
    address: Arc<Mutex<Option<String>>>,
    order_script: Arc<Script<Result<Issued, OrderError>>>,
    register_script: Arc<Script<Result<(), String>>>,
    gate: Arc<tokio::sync::Semaphore>,
    log: Arc<Mutex<Vec<String>>>,
    dir: tempfile::TempDir,
}

struct FakeOrderer {
    w: Arc<Script<Result<Issued, OrderError>>>,
    orders: Arc<AtomicUsize>,
    gate: Arc<tokio::sync::Semaphore>,
    now: Arc<AtomicI64>,
}
impl Orderer for FakeOrderer {
    fn order<'a>(&'a self, domain: &'a str, _p: &'a dyn Publisher) -> BoxFut<'a, Result<Issued, OrderError>> {
        Box::pin(async move {
            self.orders.fetch_add(1, Ordering::SeqCst);
            let _permit = self.gate.acquire().await.unwrap(); // a test can hold an order here
            let next = self.w.queue.lock().unwrap().pop_front();
            next.unwrap_or_else(|| Ok(cert(domain, self.now.load(Ordering::SeqCst) - 10, self.now.load(Ordering::SeqCst) + 90 * 86_400)))
        })
    }
}

struct FakeRegistrar {
    w: Arc<Script<Result<(), String>>>,
    seen: Arc<Mutex<Vec<String>>>,
}
impl Registrar for FakeRegistrar {
    fn register<'a>(&'a self, ip: &'a str) -> BoxFut<'a, Result<(), String>> {
        Box::pin(async move {
            self.seen.lock().unwrap().push(ip.to_string());
            self.w.queue.lock().unwrap().pop_front().unwrap_or(Ok(()))
        })
    }
}

struct FakeAddress(Arc<Mutex<Option<String>>>);
impl Addresses for FakeAddress {
    fn current(&self) -> Option<String> {
        self.0.lock().unwrap().clone()
    }
}

struct NoPublisher;
impl Publisher for NoPublisher {
    fn set<'a>(&'a self, _v: &'a str) -> BoxFut<'a, Result<(), PublishError>> {
        Box::pin(async { Ok(()) })
    }
    fn clear<'a>(&'a self, _v: &'a str) -> BoxFut<'a, Result<(), PublishError>> {
        Box::pin(async { Ok(()) })
    }
}

impl World {
    fn new() -> World {
        World {
            now: Arc::new(AtomicI64::new(T0)),
            orders: Arc::new(AtomicUsize::new(0)),
            registers: Arc::new(Mutex::new(vec![])),
            address: Arc::new(Mutex::new(Some("192.168.1.10".into()))),
            order_script: Arc::new(Script { queue: Mutex::new(VecDeque::new()) }),
            register_script: Arc::new(Script { queue: Mutex::new(VecDeque::new()) }),
            gate: Arc::new(tokio::sync::Semaphore::new(1000)),
            log: Arc::new(Mutex::new(vec![])),
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn start(&self) -> (Lan, CancellationToken) {
        let cancel = CancellationToken::new();
        let now = self.now.clone();
        let log = self.log.clone();
        let parts = Parts {
            domain: DOMAIN.into(),
            registrar: Arc::new(FakeRegistrar { w: self.register_script.clone(), seen: self.registers.clone() }),
            addresses: Arc::new(FakeAddress(self.address.clone())),
            publisher: Arc::new(NoPublisher),
            orderer: Arc::new(FakeOrderer { w: self.order_script.clone(), orders: self.orders.clone(), gate: self.gate.clone(), now: self.now.clone() }),
            files: CertFiles::new(self.dir.path(), "lan"),
            resolver: CertResolver::new(),
            clock: Arc::new(move || now.load(Ordering::SeqCst)),
            log: Arc::new(move |s| log.lock().unwrap().push(s.to_string())),
            timing: Timing { tick: Duration::from_millis(5) },
        };
        (Lan::start(parts, cancel.clone()), cancel)
    }

    fn advance(&self, secs: i64) {
        self.now.fetch_add(secs, Ordering::SeqCst);
    }
    fn orders(&self) -> usize {
        self.orders.load(Ordering::SeqCst)
    }
}

async fn until(lan: &Lan, what: &str, f: impl Fn(&Status) -> bool) -> Status {
    let mut w = lan.watch();
    let r = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let s = w.borrow_and_update().clone();
            if f(&s) {
                return s;
            }
            if w.changed().await.is_err() {
                panic!("the status channel closed while waiting for {what}");
            }
        }
    })
    .await;
    r.unwrap_or_else(|_| panic!("timed out waiting for {what}; now: {:?}", lan.status()))
}

/// A machine that follows its table never meets an event the table says cannot happen.
fn no_impossible_events(w: &World) {
    let bad: Vec<String> = w.log.lock().unwrap().iter().filter(|l| l.contains("cannot happen")).cloned().collect();
    assert!(bad.is_empty(), "the table said these cannot happen: {bad:?}");
}

/// Lets the 5 ms ticks run, then returns: for "nothing happens" checks.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(80)).await;
}

#[tokio::test]
async fn a_new_bridge_registers_then_orders_then_serves_the_certificate() {
    let w = World::new();
    let (lan, cancel) = w.start();
    let s = until(&lan, "Valid", |s| s.cert == CertState::Valid).await;
    assert_eq!(s.reg, RegState::Registered);
    assert!(s.serving() && s.describe() == "ready");
    assert!(lan.resolver.has());
    assert_eq!(s.not_after, Some(T0 + 90 * 86_400));
    assert_eq!(*w.registers.lock().unwrap(), vec!["192.168.1.10"], "registered once, with the private address");
    assert_eq!(w.orders(), 1);
    assert!(w.dir.path().join("cert-key.pem").exists(), "kept for the next start");
    no_impossible_events(&w);
    cancel.cancel();
}

#[tokio::test]
async fn the_order_waits_for_the_registration_and_the_registration_retries_on_its_schedule() {
    let w = World::new();
    *w.register_script.queue.lock().unwrap() = VecDeque::from([Err("down".into()), Err("down".into())]);
    let (lan, cancel) = w.start();
    let s = until(&lan, "RegisterWait", |s| s.reg == RegState::RegisterWait).await;
    assert_eq!(s.cert, CertState::AwaitRegistration);
    assert!(s.describe().starts_with("preparing"), "{}", s.describe());
    assert_eq!(w.orders(), 0, "no order before the name service knows us");
    settle().await;
    assert_eq!(w.registers.lock().unwrap().len(), 1, "no retry before its time: no hammering");
    w.advance(59);
    settle().await;
    assert_eq!(w.registers.lock().unwrap().len(), 1, "59 s is not 60");
    w.advance(1);
    until(&lan, "second failure", |_| w.registers.lock().unwrap().len() == 2).await;
    until(&lan, "RegisterWait again", |s| s.reg == RegState::RegisterWait).await;
    w.advance(299);
    settle().await;
    assert_eq!(w.registers.lock().unwrap().len(), 2, "the second wait is 5 minutes");
    w.advance(1);
    until(&lan, "Valid", |s| s.cert == CertState::Valid).await;
    assert_eq!(w.registers.lock().unwrap().len(), 3);
    no_impossible_events(&w);
    cancel.cancel();
}

#[tokio::test]
async fn a_rate_limit_waits_for_the_time_the_ca_named_and_the_relay_is_not_blocked() {
    let w = World::new();
    *w.order_script.queue.lock().unwrap() = VecDeque::from([Err(OrderError::RateLimited(Some(T0 + 7200)))]);
    let (lan, cancel) = w.start();
    let s = until(&lan, "OrderWait", |s| s.cert == CertState::OrderWait).await;
    assert!(s.error.contains("Let's Encrypt asked to wait"), "{:?}", s.error);
    assert!(!lan.resolver.has());
    w.advance(7199);
    settle().await;
    assert_eq!(w.orders(), 1);
    w.advance(1);
    until(&lan, "Valid", |s| s.cert == CertState::Valid).await;
    assert_eq!(w.orders(), 2);
    no_impossible_events(&w);
    cancel.cancel();
}

#[tokio::test]
async fn a_rate_limit_without_a_time_uses_the_failure_schedule_and_other_failures_back_off() {
    let w = World::new();
    *w.order_script.queue.lock().unwrap() = VecDeque::from([Err(OrderError::RateLimited(None)), Err(OrderError::Failed("boom".into())), Err(OrderError::Failed("boom".into()))]);
    let (lan, cancel) = w.start();
    until(&lan, "OrderWait", |s| s.cert == CertState::OrderWait).await;
    w.advance(60);
    until(&lan, "second order", |_| w.orders() == 2).await;
    until(&lan, "OrderWait", |s| s.cert == CertState::OrderWait && s.error.contains("boom")).await;
    w.advance(299);
    settle().await;
    assert_eq!(w.orders(), 2, "the second wait is 5 minutes (2 failures)");
    w.advance(1);
    until(&lan, "third order", |_| w.orders() == 3).await;
    w.advance(1800);
    until(&lan, "Valid", |s| s.cert == CertState::Valid).await;
    cancel.cancel();
}

#[tokio::test]
async fn renewal_starts_in_time_keeps_serving_and_replaces_the_certificate() {
    let w = World::new();
    let (lan, cancel) = w.start();
    until(&lan, "Valid", |s| s.cert == CertState::Valid).await;
    w.advance(59 * 86_400);
    settle().await;
    assert_eq!(w.orders(), 1, "30 days left is the time; 31 days left is not");
    w.advance(86_400);
    until(&lan, "the renewal order", |_| w.orders() == 2).await;
    let s = until(&lan, "Valid again", |s| s.cert == CertState::Valid && s.not_after.is_some_and(|t| t > T0 + 90 * 86_400)).await;
    assert!(s.serving() && lan.resolver.has());
    no_impossible_events(&w);
    cancel.cancel();
}

#[tokio::test]
async fn a_failing_renewal_keeps_the_old_certificate_until_it_ends_and_then_stops_serving_it() {
    let w = World::new();
    let (lan, cancel) = w.start();
    until(&lan, "Valid", |s| s.cert == CertState::Valid).await;
    *w.order_script.queue.lock().unwrap() = VecDeque::from([Err(OrderError::Failed("down".into())), Err(OrderError::Failed("down".into())), Err(OrderError::Failed("down".into()))]);
    w.advance(60 * 86_400);
    until(&lan, "RenewWait", |s| s.cert == CertState::RenewWait).await;
    assert!(lan.resolver.has() && lan.status().serving(), "still serving the valid one");
    // the certificate ends while renewal keeps failing
    w.advance(30 * 86_400 + 1);
    let s = until(&lan, "not serving", |s| !s.serving()).await;
    assert!(!lan.resolver.has(), "an expired certificate is never presented");
    assert!(matches!(s.cert, CertState::OrderWait | CertState::Ordering), "{:?}", s.cert);
    // and it recovers once the CA answers (each failure waits longer: the clock is moved past each wait)
    for _ in 0..6 {
        if lan.status().cert == CertState::Valid {
            break;
        }
        w.advance(7 * 3600);
        settle().await;
    }
    until(&lan, "Valid again", |s| s.cert == CertState::Valid).await;
    assert!(lan.resolver.has());
    no_impossible_events(&w);
    cancel.cancel();
}

#[tokio::test]
async fn a_machine_that_slept_past_the_end_of_its_certificate_stops_serving_it_and_orders_a_new_one() {
    let w = World::new();
    let (lan, cancel) = w.start();
    until(&lan, "Valid", |s| s.cert == CertState::Valid).await;
    w.advance(100 * 86_400); // renewal time and the end pass in one sleep
    until(&lan, "a second order", |_| w.orders() == 2).await;
    until(&lan, "Valid again", |s| s.cert == CertState::Valid && s.not_after.is_some_and(|t| t > T0 + 100 * 86_400)).await;
    no_impossible_events(&w);
    cancel.cancel();
}

#[tokio::test]
async fn a_restart_serves_the_saved_certificate_at_once_even_while_the_registration_is_down() {
    let w = World::new();
    let (lan, cancel) = w.start();
    until(&lan, "Valid", |s| s.cert == CertState::Valid).await;
    cancel.cancel();
    drop(lan);
    let orders_before = w.orders();
    *w.register_script.queue.lock().unwrap() = VecDeque::from([Err("down".into())]);
    let (lan2, cancel2) = w.start();
    let s = until(&lan2, "Valid", |s| s.cert == CertState::Valid).await;
    assert!(lan2.resolver.has());
    assert!(s.reg == RegState::RegisterWait || s.reg == RegState::Registering, "registration is separate: {:?}", s.reg);
    assert_eq!(w.orders(), orders_before, "no order: the saved certificate is good");
    cancel2.cancel();
}

#[tokio::test]
async fn an_expired_saved_certificate_is_not_served_and_a_new_one_is_ordered() {
    let w = World::new();
    {
        let files = CertFiles::new(w.dir.path(), "lan");
        let old = cert(DOMAIN, T0 - 200, T0 + 100);
        files.save(&old.chain_pem, &old.key_pem, DOMAIN, T0).unwrap();
    }
    w.advance(200);
    let (lan, cancel) = w.start();
    until(&lan, "Valid", |s| s.cert == CertState::Valid).await;
    assert_eq!(w.orders(), 1);
    cancel.cancel();
}

#[tokio::test]
async fn a_new_address_registers_again_and_leaves_the_certificate_alone() {
    let w = World::new();
    let (lan, cancel) = w.start();
    until(&lan, "Valid", |s| s.cert == CertState::Valid).await;
    *w.address.lock().unwrap() = Some("192.168.1.77".into());
    until(&lan, "second registration", |_| w.registers.lock().unwrap().len() == 2).await;
    until(&lan, "Registered", |s| s.reg == RegState::Registered).await;
    assert_eq!(*w.registers.lock().unwrap(), vec!["192.168.1.10", "192.168.1.77"]);
    assert_eq!(w.orders(), 1, "the certificate is for the name, not the address");
    assert_eq!(lan.status().cert, CertState::Valid);
    // the same address does not register again
    settle().await;
    assert_eq!(w.registers.lock().unwrap().len(), 2);
    no_impossible_events(&w);
    cancel.cancel();
}

#[tokio::test]
async fn no_address_is_a_failed_registration_that_ends_when_an_address_appears() {
    let w = World::new();
    *w.address.lock().unwrap() = None;
    let (lan, cancel) = w.start();
    let s = until(&lan, "RegisterWait", |s| s.reg == RegState::RegisterWait).await;
    assert!(s.error.contains("no private IPv4"), "{:?}", s.error);
    assert!(w.registers.lock().unwrap().is_empty(), "nothing was sent without an address");
    *w.address.lock().unwrap() = Some("10.1.2.3".into());
    until(&lan, "Valid", |s| s.cert == CertState::Valid).await; // no waiting for the retry time: the address is news
    assert_eq!(*w.registers.lock().unwrap(), vec!["10.1.2.3"]);
    cancel.cancel();
}

#[tokio::test]
async fn a_name_service_that_forgot_us_makes_the_bridge_register_again() {
    let w = World::new();
    *w.order_script.queue.lock().unwrap() = VecDeque::from([Err(OrderError::RegistrationLost)]);
    let (lan, cancel) = w.start();
    until(&lan, "second registration", |_| w.registers.lock().unwrap().len() >= 2).await;
    w.advance(60);
    until(&lan, "Valid", |s| s.cert == CertState::Valid).await;
    assert_eq!(w.orders(), 2);
    no_impossible_events(&w);
    cancel.cancel();
}

#[tokio::test]
async fn the_result_of_an_order_that_was_given_up_is_dropped() {
    let w = World::new();
    let hold = w.gate.clone();
    hold.forget_permits(1000); // every order waits at the gate
    let (lan, cancel) = w.start();
    until(&lan, "Ordering", |s| s.cert == CertState::Ordering).await;
    lan.disable();
    until(&lan, "Off", |s| s.cert == CertState::Off && s.reg == RegState::Off).await;
    hold.add_permits(1000); // the old order now finishes with a good certificate
    settle().await;
    let s = lan.status();
    assert_eq!(s.cert, CertState::Off, "a late result does not turn the machine back on");
    assert!(!lan.resolver.has() && !w.dir.path().join("cert-key.pem").exists(), "and nothing was kept");
    no_impossible_events(&w);
    cancel.cancel();
}

#[tokio::test]
async fn a_certificate_for_another_name_is_a_failed_order_and_is_neither_kept_nor_served() {
    let w = World::new();
    *w.order_script.queue.lock().unwrap() = VecDeque::from([Ok(cert("evil.lan.test", T0 - 10, T0 + 100_000))]);
    let (lan, cancel) = w.start();
    let s = until(&lan, "OrderWait", |s| s.cert == CertState::OrderWait).await;
    assert!(s.error.contains("not usable") && s.error.contains("does not name"), "{:?}", s.error);
    assert!(!lan.resolver.has() && !w.dir.path().join("cert-chain.pem").exists());
    cancel.cancel();
}

#[tokio::test]
async fn turning_it_off_stops_serving_and_turning_it_on_again_works() {
    let w = World::new();
    let (lan, cancel) = w.start();
    until(&lan, "Valid", |s| s.cert == CertState::Valid).await;
    lan.disable();
    until(&lan, "Off", |s| s.cert == CertState::Off && s.reg == RegState::Off).await;
    assert!(!lan.resolver.has());
    assert_eq!(lan.status().describe(), "off");
    cancel.cancel();
}

#[test]
fn renewal_time_is_30_days_before_the_end_or_a_third_of_the_life() {
    let c = Checked { not_before: 0, not_after: 90 * 86_400 };
    assert_eq!(renew_at(&c), 60 * 86_400);
    let short = Checked { not_before: 0, not_after: 6 * 86_400 };
    assert_eq!(renew_at(&short), 4 * 86_400, "a 6-day certificate renews after 4 days");
}

#[test]
fn the_failure_schedule_grows_to_six_hours_and_stays() {
    assert_eq!([0, 1, 2, 3, 4, 5, 50].map(backoff), [60, 60, 300, 1800, 7200, 21_600, 21_600]);
}

#[test]
fn the_setting_is_checked_and_old_keys_are_kept_but_unused() {
    let c: Config = serde_json::from_str(r#"{"enabled":true,"host":"rock.lan.example.com","cloudflareToken":"x"}"#).unwrap();
    assert!(c.enabled && c.valid().is_none());
    assert_eq!(c.unused_keys(), vec!["cloudflareToken", "host"]);
    assert_eq!(serde_json::to_value(&c).unwrap()["cloudflareToken"], "x", "the file is not damaged by reading it");
    let d = Config::default();
    assert!(!d.enabled && d.names == DEFAULT_NAMES && d.zone == DEFAULT_ZONE && d.valid().is_none());
    for (name, bad) in [
        ("names", Config { names: "http://names.example".into(), ..Default::default() }),
        ("zone", Config { zone: "x".into(), ..Default::default() }),
        ("ca", Config { ca: "prod".into(), ..Default::default() }),
        ("email", Config { email: "a b".into(), ..Default::default() }),
    ] {
        assert!(bad.valid().is_some_and(|m| m.contains(name)), "{name}");
    }
}

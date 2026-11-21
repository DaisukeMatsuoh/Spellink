//! Wires the bridge together and runs it (Go: main.go's serve, devices.go): the addresses a device
//! is told, the ledger and its relay rooms, the status the app page shows, and `serve` itself.

use crate::config::{Config, Store as ConfigStore};
use crate::connector::{self, RoomSet, ServeFn};
use crate::devices::{self, FileStore, Hooks, Registry, registry::PackageData};
use crate::glyphs::{self, Font};
use crate::history;
use crate::protocol::{self, Settings};
use crate::provider::{Provider, claudecode};
use crate::relay::room_id;
use crate::server::admin::{AdminDeps, ProviderInfo, StatusInfo};
use crate::server::run::Runs;
use crate::server::session::{Session, Sessions};
use crate::server::transport::Transport;
use crate::server::{Deps, LogFn, http};
use crate::ui;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn stdout_log() -> LogFn {
    Arc::new(|s| println!("{} {s}", crate::util::now_rfc3339()))
}

/// The addresses a device is told to use with `secret`: every wss:// address the bridge has (the
/// plugin refuses anything else), each naming the device, and the relay room that the secret
/// opens. No address at all is not an error here; the package builder says so.
pub fn device_urls(cfg: &Config, id: &str, secret: &str) -> Vec<String> {
    let named = |base: &str| -> Option<String> {
        let mut u = url::Url::parse(base).ok().filter(|u| u.scheme() == "wss")?;
        let mut q: Vec<(String, String)> = u.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).filter(|(k, _)| k != "d").collect();
        q.push(("d".into(), id.to_string()));
        q.sort(); // Go's Values.Encode sorts by key
        u.query_pairs_mut().clear().extend_pairs(q);
        Some(u.to_string())
    };
    let mut urls = vec![];
    if let Some(lan) = cfg.lan.as_ref().filter(|l| l.enabled && !l.host.is_empty()) {
        urls.extend(named(&format!("wss://{}:8766{}", lan.host, protocol::WS_PATH)));
    }
    urls.extend(cfg.extra_urls.iter().filter_map(|u| named(u)));
    if !cfg.relay_url.is_empty() && !secret.is_empty() {
        urls.push(crate::connector::client_url(&cfg.relay_url, secret));
    }
    urls
}

/// The set of relay rooms to be registered in: one for every secret that may connect, each served
/// as the device it belongs to.
pub fn rooms_for(secrets: &[devices::Secret], serve: impl Fn(&str) -> ServeFn) -> HashMap<String, ServeFn> {
    secrets.iter().map(|s| (room_id(&s.secret), serve(&s.id))).collect()
}

/// The name a device shows for this bridge; one that is not usable (empty, too long) is "bridge".
pub fn bridge_name(cfg: &Config) -> String {
    if devices::valid_name(&cfg.name) { cfg.name.clone() } else { "bridge".into() }
}

const NO_BASE_PACKAGE: &str = "set pairing.pluginPackage in config.json to the path of the plugin package (Spellink.snplg) to build setup files from";

/// The function that builds a device's setup file from the plugin package.
pub fn package_builder(cfg: Arc<Config>) -> Arc<dyn Fn(PackageData) -> Result<Vec<u8>, String> + Send + Sync> {
    Arc::new(move |mut d: PackageData| {
        let path = cfg.plugin_package();
        if path.is_empty() {
            return Err(NO_BASE_PACKAGE.into());
        }
        let base = std::fs::read(path).map_err(|e| format!("reading {path}: {e}"))?;
        // the setup names the BRIDGE (what the device shows); the device's own name stays in the ledger
        d.name = bridge_name(&cfg);
        let urls = device_urls(&cfg, &d.id, &d.b);
        devices::pkg::build_package(&base, &d, &urls)
    })
}

/// Where a program is: a path with a separator is looked at as it is, a bare name is searched for
/// along PATH (and, on Windows, with the usual extensions).
pub fn look_path(exe: &str) -> Option<PathBuf> {
    let is_file = |p: &Path| p.is_file() && is_executable(p);
    if exe.contains('/') || exe.contains('\\') {
        let p = PathBuf::from(exe);
        return is_file(&p).then_some(p);
    }
    let exts: Vec<String> = if cfg!(windows) { std::env::var("PATHEXT").unwrap_or(".EXE;.CMD;.BAT".into()).split(';').map(String::from).chain([String::new()]).collect() } else { vec![String::new()] };
    for dir in std::env::split_paths(&std::env::var_os("PATH")?) {
        for ext in &exts {
            let p = dir.join(format!("{exe}{ext}"));
            if is_file(&p) {
                return Some(p);
            }
        }
    }
    None
}

fn is_executable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        let _ = p;
        true
    }
}

/// What the app page shows about the bridge: its name, the AI program and where it is, and the
/// addresses a new device would be told.
pub fn status_func(cfg: Arc<Config>, ai_name: String) -> Arc<dyn Fn() -> StatusInfo + Send + Sync> {
    Arc::new(move || {
        let (_, _, exe) = cfg.provider_settings();
        let mut info = StatusInfo {
            version: VERSION.into(),
            name: bridge_name(&cfg),
            relay: !cfg.relay_url.is_empty(),
            addresses: device_urls(&cfg, "<id>", ""),
            provider: ProviderInfo { name: ai_name.clone(), ..ProviderInfo::default() },
        };
        if let Some(p) = look_path(&exe) {
            info.provider.found = true;
            info.provider.path = p.to_string_lossy().into_owned();
        }
        info
    })
}

/// For the log: the addresses a new device would be told.
pub fn describe_urls(cfg: &Config) -> String {
    let u = device_urls(cfg, "<id>", "");
    if u.is_empty() { "none (add a wss:// address: extraUrls, lan.enabled or relayUrl)".into() } else { u.join(", ") }
}

/// Every LAN IPv4 address plus the mDNS host name; the plugin tries them all.
pub fn candidate_urls(port: i64) -> Vec<String> {
    let mut hosts: Vec<String> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback())
        .filter_map(|i| match i.ip() {
            std::net::IpAddr::V4(v4) => Some(v4.to_string()),
            _ => None,
        })
        .collect();
    let host = crate::config::hostname();
    if !host.is_empty() {
        hosts.push(format!("{host}.local"));
    }
    hosts.into_iter().map(|h| format!("ws://{h}:{port}{}", protocol::WS_PATH)).collect()
}

/// What the plugin tries in parallel: LAN addresses first, then configured extras.
pub fn pair_urls(cfg: &Config) -> Vec<String> {
    let mut urls = candidate_urls(cfg.port);
    urls.extend(cfg.extra_urls.iter().cloned());
    if !cfg.relay_url.is_empty() {
        urls.push(crate::connector::client_url(&cfg.relay_url, &cfg.token));
    }
    if let Some(lan) = cfg.lan.as_ref().filter(|l| l.enabled && !l.host.is_empty()) {
        urls.insert(0, format!("wss://{}:8766{}", lan.host, protocol::WS_PATH));
    }
    urls
}

fn clamp(v: i64, lo: i64, hi: i64) -> i64 {
    v.clamp(lo, hi)
}

/// Reads the glyph data, building it the first time (a missing file or one of an older format).
pub fn load_font(store: &ConfigStore) -> Result<Arc<Font>, String> {
    let file = store.glyph_file();
    match std::fs::File::open(&file) {
        Ok(f) => match glyphs::read_font(std::io::BufReader::new(f)) {
            Ok(font) => return Ok(Arc::new(font)),
            Err(glyphs::FontError::Format) => {}
            Err(e) => return Err(format!("{}: {e}", file.display())),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("{}: {e}", file.display())),
    }
    crate::glyphbuild::command(store)?;
    let f = std::fs::File::open(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    glyphs::read_font(std::io::BufReader::new(f)).map(Arc::new).map_err(|e| format!("{}: {e}", file.display()))
}

/// Builds the AI the config names. Adding another AI is a case here and a module under provider;
/// nothing else in the bridge changes (ADR 0015).
pub fn new_provider(cfg: &Config) -> Result<Arc<dyn Provider>, String> {
    let (kind, model, executable) = cfg.provider_settings();
    match kind.as_str() {
        "claude-code" => Ok(Arc::new(claudecode::Runner::new(&executable, &model)?)),
        other => Err(format!("unknown provider {other:?}")), // Config::validate already refused it
    }
}

/// Runs the bridge until SIGINT/SIGTERM.
pub async fn serve() -> Result<(), String> {
    let store = ConfigStore::user()?;
    let mut cfg = store.load()?;
    let log = stdout_log();
    let lan_identity = prepare_lan(&mut cfg, store.dir(), &log)?;
    let font = {
        let store = store.clone();
        tokio::task::spawn_blocking(move || load_font(&store)).await.map_err(|e| e.to_string())??
    };
    let ai = new_provider(&cfg)?;
    let (first, gap, total) = cfg.provider_timeouts();
    let notes = if cfg.history_chars() > 0 { Arc::new(history::Store::new(store.history_dir())) } else { Arc::new(history::Store::none()) };
    let shared = Arc::new(Mutex::new(cfg.clone()));
    let (cfg_for_settings, store_for_settings, log_for_settings) = (shared.clone(), store.clone(), log.clone());
    let (cfg_for_set, store_for_set) = (shared.clone(), store.clone());
    let (store_for_report, log_for_report) = (store.clone(), log.clone());
    let settings: Arc<dyn Fn() -> Settings + Send + Sync> = Arc::new(move || {
        let c = cfg_for_settings.lock().unwrap();
        Settings {
            show_progress: c.show_progress,
            line_tilt: c.line_tilt_on(),
            tilt_degrees: c.tilt_degrees.unwrap_or(0.0),
            thinking_animation: c.thinking_animation_on(),
            full_width_spacing: c.full_spacing(),
            half_width_spacing: c.half_spacing(),
            avoid_overwriting: c.avoid_overwriting,
            busy_notice: c.busy_notice_on(),
        }
    });
    let _ = (&store_for_settings, &log_for_settings);
    let set_settings: Arc<dyn Fn(Settings) -> Result<(), String> + Send + Sync> = {
        let log = log.clone();
        Arc::new(move |s: Settings| {
            let mut c = cfg_for_set.lock().unwrap();
            c.show_progress = s.show_progress;
            c.line_tilt = Some(s.line_tilt);
            c.thinking_animation = Some(s.thinking_animation);
            let full = clamp(s.full_width_spacing, protocol::SPACING_MIN, protocol::FULL_SPACING_MAX);
            let half = clamp(s.half_width_spacing, protocol::SPACING_MIN, protocol::HALF_SPACING_MAX);
            c.full_width_spacing = Some(full);
            c.half_width_spacing = Some(half);
            c.busy_notice = Some(s.busy_notice);
            c.avoid_overwriting = s.avoid_overwriting;
            log(&format!("settings changed: showProgress={} lineTilt={} animation={} spacing={full}/{half}", s.show_progress, s.line_tilt, s.thinking_animation));
            store_for_set.save(&c)
        })
    };
    let report: Arc<dyn Fn(&str, Option<&serde_json::Value>) + Send + Sync> = Arc::new(move |peer, data| {
        if let Err(e) = store_for_report.append_report(peer, data.unwrap_or(&serde_json::Value::Null)) {
            log_for_report(&format!("report: {e}"));
        }
    });

    let cancel = CancellationToken::new();
    let sessions = Arc::new(Sessions::default());
    let deps_cell: Arc<OnceLock<Arc<Deps>>> = Arc::new(OnceLock::new());
    let (registry, rooms) = start_devices(&cancel, &store, &cfg, &log, &sessions, &deps_cell)?;
    let admin_token = store.admin_token()?;
    let cfg_arc = Arc::new(cfg.clone());
    let deps = Arc::new(Deps {
        name: cfg.name.clone(),
        version: VERSION.into(),
        token: cfg.token.clone(),
        font: Some(font),
        provider: ai.clone(),
        report,
        log: log.clone(),
        settings,
        set_settings,
        history: notes,
        history_chars: cfg.history_chars(),
        runs: Arc::new(Runs::default()),
        first_output: first,
        gap,
        total,
        keepalive_every: Duration::ZERO,
        orphan_grace: Duration::ZERO,
        devices: Some(registry.clone()),
        sessions: sessions.clone(),
        pair_request_wait: Duration::ZERO,
        pairing_max: Duration::ZERO,
    });
    deps_cell.set(deps.clone()).map_err(|_| "the server is already set up".to_string())?;
    if let Some(rooms) = &rooms {
        sync_rooms(rooms, &registry, &deps_cell);
    }
    let admin = AdminDeps { devices: registry, token: admin_token, package: package_builder(cfg_arc.clone()), status: Some(status_func(cfg_arc.clone(), ai.name())) };
    let (pages, problems) = ui::pages();
    for p in problems {
        log(&p);
    }
    let router = http::router(deps.clone(), Some(admin), Some(pages));

    if !cfg.relay_url.is_empty() {
        let serve_fn: ServeFn = {
            let deps = deps.clone();
            Arc::new(move |t: Transport, peer: String| {
                let deps = deps.clone();
                Box::pin(async move { Session::serve(deps, t, &peer, true, "").await })
            })
        };
        tokio::spawn(connector::connect(cancel.clone(), cfg.relay_url.clone(), cfg.token.clone(), serve_fn, log.clone()));
    }
    if let (Some(identity), Some(l)) = (lan_identity, cfg.lan.clone()) {
        start_lan(&l, identity, store.dir(), &log, router.clone(), &cancel).await;
    }

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", cfg.port as u16)).await.map_err(|e| format!("port {}: {e}", cfg.port))?;
    log(&format!("spellink-bridge {VERSION} {:?} listening on port {} (config: {})", cfg.name, cfg.port, store.dir().display()));
    for u in pair_urls(&cfg) {
        log(&format!("  {u}"));
    }
    log(&format!("devices are told: {}", describe_urls(&cfg)));
    let app = router.into_make_service_with_connect_info::<SocketAddr>();
    let stop = cancel.clone();
    axum::serve(crate::util::low_latency(listener), app)
        .with_graceful_shutdown(async move {
            wait_for_signal().await;
            stop.cancel();
        })
        .await
        .map_err(|e| e.to_string())
}

/// Reads the `lan` setting. When it is on and usable, makes (or reads) this bridge's identity and
/// puts its name in the setting, so that devices are told it. A setting that is not usable is
/// reported and turned off for this run; it is not a reason to stop the bridge (the relay and
/// the tunnel still work).
fn prepare_lan(cfg: &mut Config, dir: &std::path::Path, log: &LogFn) -> Result<Option<Arc<crate::namesproto::Identity>>, String> {
    let Some(l) = cfg.lan.as_mut().filter(|l| l.enabled) else { return Ok(None) };
    if let Some(why) = l.valid() {
        log(&format!("lan: not used: {why}"));
        l.enabled = false;
        return Ok(None);
    }
    let unused = l.unused_keys().join(", ");
    if !unused.is_empty() {
        log(&format!("lan: the settings {unused} are not used any more (the name service replaces them); they can be removed"));
    }
    let identity = Arc::new(crate::namesproto::Identity::load_or_create(dir)?);
    l.host = format!("{}.{}", identity.id(), l.zone.trim_end_matches('.').to_ascii_lowercase());
    Ok(Some(identity))
}

/// Registers with the name service, gets the certificate, and serves the same pages on the LAN
/// port with it (8766). Nothing here stops the bridge: if it cannot start, the log says why.
async fn start_lan(l: &crate::lan::Config, identity: Arc<crate::namesproto::Identity>, dir: &std::path::Path, log: &LogFn, router: axum::Router, cancel: &CancellationToken) {
    use crate::lan::{Lan, LanAddress, Parts, Timing, client::NamesClient, system_clock};
    let clock = system_clock();
    let client = match NamesClient::new(&l.names, identity, clock.clone()) {
        Ok(c) => Arc::new(c),
        Err(e) => return log(&format!("lan: not started: {e}")),
    };
    let listener = match tokio::net::TcpListener::bind(("0.0.0.0", LAN_PORT)).await {
        Ok(l) => l,
        Err(e) => return log(&format!("lan: not started: port {LAN_PORT}: {e}")),
    };
    let resolver = crate::tlsserve::CertResolver::new();
    let log_for_lan = log.clone();
    let parts = Parts {
        domain: l.host.clone(),
        registrar: client.clone(),
        addresses: Arc::new(LanAddress),
        publisher: client,
        orderer: Arc::new(crate::acme::Acme::new(dir, l.directory(), &l.email)),
        files: crate::acme::CertFiles::new(dir, "lan"),
        resolver: resolver.clone(),
        clock,
        log: Arc::new(move |s| log_for_lan(s)),
        timing: Timing::default(),
    };
    let lan = Lan::start(parts, cancel.clone());
    log(&format!("lan: {} on port {LAN_PORT}", l.host));
    let (tls_cancel, tls_log) = (cancel.clone(), log.clone());
    tokio::spawn(async move {
        if let Err(e) = crate::tlsserve::serve(listener, router, resolver, tls_cancel).await {
            tls_log(&format!("lan: stopped: {e}"));
        }
    });
    let (mut watch, status_log, stop) = (lan.watch(), log.clone(), cancel.clone());
    tokio::spawn(async move {
        let _keep = &lan; // the machines run while the bridge does
        let mut last = String::new();
        loop {
            let now = watch.borrow_and_update().describe();
            if now != last {
                status_log(&format!("lan: {now}"));
                last = now;
            }
            tokio::select! {
                _ = stop.cancelled() => return,
                r = watch.changed() => if r.is_err() { return },
            }
        }
    });
}

pub const LAN_PORT: u16 = 8766;

pub async fn wait_for_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("SIGTERM handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

fn sync_rooms(rooms: &RoomSet, reg: &Registry, deps: &Arc<OnceLock<Arc<Deps>>>) {
    let deps = deps.clone();
    rooms.sync(rooms_for(&reg.active_secrets(), move |id| {
        let (deps, id) = (deps.clone(), id.to_string());
        Arc::new(move |t: Transport, peer: String| {
            let (deps, id) = (deps.clone(), id.clone());
            Box::pin(async move {
                if let Some(d) = deps.get() {
                    Session::serve(d.clone(), t, &peer, true, &id).await;
                }
            })
        })
    }));
}

/// Opens the ledger and starts what keeps it right: the timers, and the relay rooms that follow
/// the devices.
fn start_devices(cancel: &CancellationToken, store: &ConfigStore, cfg: &Config, log: &LogFn, sessions: &Arc<Sessions>, deps: &Arc<OnceLock<Arc<Deps>>>) -> Result<(Registry, Option<Arc<RoomSet>>), String> {
    let file = store.devices_file();
    let rooms = (!cfg.relay_url.is_empty()).then(|| Arc::new(RoomSet::new(cancel.clone(), &cfg.relay_url, log.clone())));
    let reg_cell: Arc<OnceLock<Registry>> = Arc::new(OnceLock::new());
    let changed: Arc<dyn Fn() + Send + Sync> = {
        let (rooms, reg_cell, deps) = (rooms.clone(), reg_cell.clone(), deps.clone());
        Arc::new(move || {
            if let (Some(rooms), Some(reg)) = (&rooms, reg_cell.get()) {
                sync_rooms(rooms, reg, &deps);
            }
        })
    };
    let (cfg_ep, s2) = (Arc::new(cfg.clone()), sessions.clone());
    let hooks = Hooks { endpoints: Some(Arc::new(move |id, k| device_urls(&cfg_ep, id, k))), changed: Some(changed), close_all: Some(Arc::new(move |id| s2.close_device(id))) };
    let log2 = log.clone();
    let dev_cfg = devices::Config { window: cfg.pairing_window(), ttl: cfg.pairing_ttl(), log: Some(Arc::new(move |s| log2(s))), ..devices::Config::default() };
    let (reg, skipped) = Registry::new(dev_cfg, Box::new(FileStore::new(&file)), hooks, devices::os_rand()).map_err(|e| format!("{}: {e}", file.display()))?;
    for s in skipped {
        log(&format!("devices: {}: skipped {s}", file.display()));
    }
    reg_cell.set(reg.clone()).map_err(|_| "the ledger is already set up".to_string())?;
    let (r2, c2) = (reg.clone(), cancel.clone());
    tokio::spawn(async move {
        let mut t = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = t.tick() => r2.tick(),
                _ = c2.cancelled() => return,
            }
        }
    });
    Ok((reg, rooms))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lan;

    fn cfg() -> Config {
        Config { name: "Rock".into(), port: 8765, token: "tok".into(), ..Config::default() }
    }

    #[test]
    fn a_device_is_told_only_wss_addresses_each_naming_it() {
        let mut c = cfg();
        c.extra_urls = vec!["wss://bridge.example.com/v1".into(), "ws://plain.example/v1".into(), "https://nope/v1".into(), "wss://other.example/v1?x=1&d=old".into()];
        c.relay_url = "wss://relay.example.com".into();
        c.lan = Some(lan::Config { enabled: true, host: "rock.lan.example.com".into(), ..Default::default() });
        let urls = device_urls(&c, "0123456789abcdef", "SECRET");
        assert_eq!(
            urls,
            vec![
                "wss://rock.lan.example.com:8766/v1?d=0123456789abcdef".to_string(),
                "wss://bridge.example.com/v1?d=0123456789abcdef".into(),
                "wss://other.example/v1?d=0123456789abcdef&x=1".into(),
                format!("wss://relay.example.com/v1/relay/client?room={}", room_id("SECRET")),
            ]
        );
        assert!(device_urls(&cfg(), "0123456789abcdef", "SECRET").is_empty(), "no address at all is not an error here");
        assert_eq!(device_urls(&c, "0123456789abcdef", "").len(), 3, "without a secret there is no relay room to name");
    }

    #[test]
    fn the_bridge_name_falls_back_to_bridge() {
        assert_eq!(bridge_name(&cfg()), "Rock");
        for bad in ["", &"x".repeat(65), "tab\there"] {
            assert_eq!(bridge_name(&Config { name: bad.into(), ..cfg() }), "bridge", "{bad:?}");
        }
    }

    #[test]
    fn the_package_builder_says_what_is_missing() {
        let build = package_builder(Arc::new(cfg()));
        let d = PackageData { id: "0123456789abcdef".into(), name: "n".into(), generation: 1, b: String::new(), window: Duration::from_secs(90) };
        assert!(build(d.clone()).unwrap_err().contains("pluginPackage"));
        let c = Config { pairing: Some(crate::config::Pairing { plugin_package: "/nonexistent/x.snplg".into(), ..Default::default() }), ..cfg() };
        assert!(package_builder(Arc::new(c))(d).unwrap_err().contains("/nonexistent/x.snplg"));
    }

    #[test]
    fn the_rooms_are_one_per_secret() {
        let secrets = vec![devices::Secret { id: "a".into(), secret: "s1".into(), is_k: false }, devices::Secret { id: "a".into(), secret: "s2".into(), is_k: true }];
        let want = rooms_for(&secrets, |_| Arc::new(|_, _| Box::pin(async {})));
        let mut keys: Vec<_> = want.keys().cloned().collect();
        keys.sort();
        let mut expect = vec![room_id("s1"), room_id("s2")];
        expect.sort();
        assert_eq!(keys, expect);
    }

    #[test]
    fn a_program_is_found_on_the_path_or_by_its_own() {
        assert!(look_path("sh").is_some() || cfg!(windows));
        assert!(look_path("definitely-not-a-program-xyz").is_none());
        assert!(look_path("/nonexistent/claude").is_none());
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("prog");
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert!(look_path(p.to_str().unwrap()).is_none(), "a file that is not executable is not a program");
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert_eq!(look_path(p.to_str().unwrap()), Some(p));
        }
    }

    #[test]
    fn the_pair_urls_put_the_wss_lan_name_first() {
        let c = Config { relay_url: "wss://r.example".into(), extra_urls: vec!["wss://t.example/v1".into()], lan: Some(lan::Config { enabled: true, host: "h.example.com".into(), ..Default::default() }), ..cfg() };
        let urls = pair_urls(&c);
        assert_eq!(urls[0], "wss://h.example.com:8766/v1");
        assert!(urls.iter().any(|u| u == "wss://t.example/v1") && urls.last().unwrap().contains("/v1/relay/client?room="));
        assert!(urls.iter().any(|u| u.starts_with("ws://") && u.contains(":8765/v1")), "the LAN addresses and the .local name");
    }

    #[test]
    fn the_status_names_the_ai_and_where_it_is() {
        let c = Arc::new(Config { claude_executable: "sh".into(), ..cfg() });
        let info = status_func(c, "Claude Code".into())();
        assert_eq!((info.name.as_str(), info.provider.name.as_str(), info.relay), ("Rock", "Claude Code", false));
        assert!(info.provider.found == look_path("sh").is_some());
    }
}

//! What the relay counts about its own use (ADR 0021): durations and numbers, in fixed-width
//! histograms, one set per UTC day, the last `KEEP_DAYS` days. No address, room, session id or
//! time of day is kept: the relay cannot read the traffic and does not note who sent it.
//! `GET /v1/stats` returns this; a weekly job adds the histograms of all relays together.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

pub const KEEP_DAYS: usize = 8;
/// Upper edges of the buckets; a value goes in the first bucket whose edge is not below it, and a
/// last bucket takes everything above the final edge. Part of the published format (interface I17).
pub const BOUNDS_MS: [u64; 9] = [10, 20, 50, 100, 200, 500, 1000, 2000, 5000];
pub const BOUNDS_S: [u64; 8] = [10, 30, 60, 120, 300, 600, 1800, 3600];
pub const BOUNDS_BYTES: [u64; 5] = [1 << 10, 10 << 10, 100 << 10, 1 << 20, 10 << 20];
/// The reasons a connection is turned away; anything else is counted as "other".
pub const REFUSED: [&str; 7] = ["rate", "full", "offline", "busy", "unanswered", "bad_room", "other"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Histogram(Vec<u64>);

impl Histogram {
    fn new(edges: usize) -> Histogram {
        Histogram(vec![0; edges + 1])
    }
    fn record(&mut self, bounds: &[u64], v: u64) {
        let i = bounds.iter().position(|&b| v <= b).unwrap_or(bounds.len());
        self.0[i] = self.0[i].saturating_add(1);
    }
    pub fn counts(&self) -> &[u64] {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Day {
    pub day: String,
    pub sessions: u64,
    pub peak_concurrent: u64,
    pub refused: BTreeMap<String, u64>,
    pub ping_client_ms: Histogram,
    pub ping_bridge_ms: Histogram,
    pub forward_ms: Histogram,
    pub setup_ms: Histogram,
    pub session_s: Histogram,
    pub session_bytes: Histogram,
}

impl Day {
    fn new(day: String) -> Day {
        Day {
            day,
            sessions: 0,
            peak_concurrent: 0,
            refused: REFUSED.iter().map(|r| (r.to_string(), 0)).collect(),
            ping_client_ms: Histogram::new(BOUNDS_MS.len()),
            ping_bridge_ms: Histogram::new(BOUNDS_MS.len()),
            forward_ms: Histogram::new(BOUNDS_MS.len()),
            setup_ms: Histogram::new(BOUNDS_MS.len()),
            session_s: Histogram::new(BOUNDS_S.len()),
            session_bytes: Histogram::new(BOUNDS_BYTES.len()),
        }
    }

    /// True if every histogram has the length the bounds give and every refusal reason is known:
    /// a stored file is a boundary like any other.
    fn well_formed(&self) -> bool {
        let ok = |h: &Histogram, n: usize| h.0.len() == n + 1;
        ok(&self.ping_client_ms, BOUNDS_MS.len())
            && ok(&self.ping_bridge_ms, BOUNDS_MS.len())
            && ok(&self.forward_ms, BOUNDS_MS.len())
            && ok(&self.setup_ms, BOUNDS_MS.len())
            && ok(&self.session_s, BOUNDS_S.len())
            && ok(&self.session_bytes, BOUNDS_BYTES.len())
            && self.refused.keys().all(|k| REFUSED.contains(&k.as_str()))
            && civil_ok(&self.day)
    }
}

fn civil_ok(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10 && b[4] == b'-' && b[7] == b'-' && b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

/// `YYYY-MM-DD` of a day number (days since 1970-01-01).
pub fn day_name(days: u64) -> String {
    // civil-from-days (Howard Hinnant)
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02}", if m <= 2 { y + 1 } else { y }, m, d)
}

#[derive(Default)]
struct Inner {
    days: Vec<Day>,
    concurrent: u64,
}

pub struct Stats(Mutex<Inner>);

#[derive(Serialize, Deserialize)]
struct Published {
    v: u32,
    bounds_ms: Vec<u64>,
    bounds_s: Vec<u64>,
    bounds_bytes: Vec<u64>,
    days: Vec<Day>,
}

impl Default for Stats {
    fn default() -> Self {
        Stats::new()
    }
}

impl Stats {
    pub fn new() -> Stats {
        Stats(Mutex::new(Inner::default()))
    }

    /// The day record for `day` (a day number), created and the oldest dropped as needed.
    fn today(g: &mut Inner, day: u64) -> &mut Day {
        let name = day_name(day);
        if !g.days.iter().any(|d| d.day == name) {
            g.days.push(Day::new(name.clone()));
            g.days.sort_by(|a, b| a.day.cmp(&b.day));
            while g.days.len() > KEEP_DAYS {
                g.days.remove(0);
            }
        }
        g.days.iter_mut().find(|d| d.day == name).expect("just inserted")
    }

    fn with(&self, day: u64, f: impl FnOnce(&mut Day)) {
        let mut g = self.0.lock().unwrap();
        f(Self::today(&mut g, day));
    }

    pub fn ping_client(&self, day: u64, ms: u64) {
        self.with(day, |d| d.ping_client_ms.record(&BOUNDS_MS, ms));
    }
    pub fn ping_bridge(&self, day: u64, ms: u64) {
        self.with(day, |d| d.ping_bridge_ms.record(&BOUNDS_MS, ms));
    }
    pub fn forward(&self, day: u64, ms: u64) {
        self.with(day, |d| d.forward_ms.record(&BOUNDS_MS, ms));
    }
    pub fn refused(&self, day: u64, reason: &str) {
        let key = if REFUSED.contains(&reason) { reason } else { "other" };
        self.with(day, |d| {
            let c = d.refused.entry(key.to_string()).or_insert(0);
            *c = c.saturating_add(1);
        });
    }

    /// A session has been piped through: `setup_ms` is how long the plugin waited for it.
    pub fn session_start(&self, day: u64, setup_ms: u64) {
        let mut g = self.0.lock().unwrap();
        g.concurrent += 1;
        let now = g.concurrent;
        let d = Self::today(&mut g, day);
        d.sessions = d.sessions.saturating_add(1);
        d.peak_concurrent = d.peak_concurrent.max(now);
        d.setup_ms.record(&BOUNDS_MS, setup_ms);
    }

    pub fn session_end(&self, day: u64, seconds: u64, bytes: u64) {
        let mut g = self.0.lock().unwrap();
        g.concurrent = g.concurrent.saturating_sub(1);
        let d = Self::today(&mut g, day);
        d.session_s.record(&BOUNDS_S, seconds);
        d.session_bytes.record(&BOUNDS_BYTES, bytes);
    }

    pub fn json(&self) -> String {
        let g = self.0.lock().unwrap();
        let p = Published { v: 1, bounds_ms: BOUNDS_MS.to_vec(), bounds_s: BOUNDS_S.to_vec(), bounds_bytes: BOUNDS_BYTES.to_vec(), days: g.days.clone() };
        serde_json::to_string(&p).expect("stats serialize")
    }

    /// Writes the days to `path` (0600, through a temporary file).
    pub fn save(&self, path: &Path) -> Result<(), String> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let text = self.json();
        let tmp = path.with_extension("tmp");
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
        f.write_all(text.as_bytes()).and_then(|_| f.sync_all()).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Reads what `save` wrote. A missing file is an empty start; a damaged one is refused whole
    /// (a half-trusted file would put wrong numbers in the published histograms).
    pub fn load(path: &Path) -> Result<Stats, String> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Stats::new()),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let p: Published = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        if p.v != 1 || p.bounds_ms != BOUNDS_MS || p.bounds_s != BOUNDS_S || p.bounds_bytes != BOUNDS_BYTES {
            return Err(format!("{}: written with other bounds", path.display()));
        }
        if p.days.len() > KEEP_DAYS || !p.days.iter().all(Day::well_formed) {
            return Err(format!("{}: not a stats file", path.display()));
        }
        Ok(Stats(Mutex::new(Inner { days: p.days, concurrent: 0 })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_goes_in_the_first_bucket_whose_edge_it_does_not_pass() {
        let s = Stats::new();
        for ms in [0, 10, 11, 20, 4999, 5000, 5001, u64::MAX] {
            s.forward(20_000, ms);
        }
        let g = s.0.lock().unwrap();
        assert_eq!(g.days[0].forward_ms.counts(), &[2, 2, 0, 0, 0, 0, 0, 0, 2, 2]);
    }

    #[test]
    fn days_are_named_in_utc_and_only_the_last_eight_are_kept() {
        assert_eq!(day_name(0), "1970-01-01");
        assert_eq!(day_name(20_088), "2024-12-31");
        assert_eq!(day_name(20_089), "2025-01-01");
        assert_eq!(day_name(20_454), "2026-01-01");
        let s = Stats::new();
        for d in 20_000..20_012 {
            s.ping_client(d, 5);
        }
        let g = s.0.lock().unwrap();
        assert_eq!(g.days.len(), KEEP_DAYS);
        assert_eq!(g.days.first().unwrap().day, day_name(20_004));
        assert_eq!(g.days.last().unwrap().day, day_name(20_011));
    }

    #[test]
    fn sessions_count_the_peak_and_the_two_histograms_at_the_end() {
        let s = Stats::new();
        s.session_start(20_000, 120);
        s.session_start(20_000, 30);
        s.session_end(20_000, 5, 2048);
        s.session_start(20_000, 40);
        s.session_end(20_000, 4000, 50 << 20);
        s.session_end(20_000, 0, 0);
        let g = s.0.lock().unwrap();
        let d = &g.days[0];
        assert_eq!((d.sessions, d.peak_concurrent), (3, 2));
        assert_eq!(g.concurrent, 0);
        assert_eq!(d.setup_ms.counts().iter().sum::<u64>(), 3);
        assert_eq!(d.session_s.counts()[0], 2); // 5 s and 0 s
        assert_eq!(*d.session_s.counts().last().unwrap(), 1); // 4000 s
        assert_eq!(*d.session_bytes.counts().last().unwrap(), 1);
    }

    #[test]
    fn an_end_without_a_start_does_not_go_below_zero() {
        let s = Stats::new();
        s.session_end(20_000, 1, 1);
        assert_eq!(s.0.lock().unwrap().concurrent, 0);
    }

    #[test]
    fn an_unknown_refusal_is_counted_as_other() {
        let s = Stats::new();
        s.refused(20_000, "offline");
        s.refused(20_000, "made up by a caller");
        let g = s.0.lock().unwrap();
        assert_eq!(g.days[0].refused["offline"], 1);
        assert_eq!(g.days[0].refused["other"], 1);
        assert!(g.days[0].refused.keys().all(|k| REFUSED.contains(&k.as_str())));
    }

    #[test]
    fn what_is_published_is_numbers_only_and_reads_back() {
        let s = Stats::new();
        s.ping_client(20_000, 33);
        s.session_start(20_000, 80);
        let text = s.json();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["v"], 1);
        assert_eq!(v["days"][0]["ping_client_ms"].as_array().unwrap().len(), BOUNDS_MS.len() + 1);
        // nothing but the day, counters, and histograms: no field could hold an address or a name
        let keys: Vec<&str> = v["days"][0].as_object().unwrap().keys().map(String::as_str).collect();
        for k in &keys {
            assert!(["day", "sessions", "peak_concurrent", "refused", "ping_client_ms", "ping_bridge_ms", "forward_ms", "setup_ms", "session_s", "session_bytes"].contains(k), "{k}");
        }
    }

    #[test]
    fn it_survives_a_restart_and_refuses_a_damaged_file() {
        let dir = std::env::temp_dir().join(format!("spellink-stats-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("stats.json");
        assert!(Stats::load(&path).unwrap().0.lock().unwrap().days.is_empty());
        let s = Stats::new();
        s.forward(20_000, 7);
        s.save(&path).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let back = Stats::load(&path).unwrap();
        assert_eq!(back.json(), s.json());
        // a histogram of the wrong length, an unknown reason, a bad day, other bounds, not json
        let text = std::fs::read_to_string(&path).unwrap();
        for bad in [text.replace("\"forward_ms\":[1,", "\"forward_ms\":[1,0,"), text.replace("\"offline\"", "\"x\""), text.replace("2024-", "20x4-"), text.replace("[10,20,", "[11,20,"), "{".to_string()] {
            std::fs::write(&path, bad).unwrap();
            assert!(Stats::load(&path).is_err());
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    use std::os::unix::fs::PermissionsExt;
}

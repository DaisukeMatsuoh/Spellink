//! A point in time as the ledger stores it: RFC 3339 text, like Go's `time.Time` (so a ledger
//! written by the Go bridge reads here, offsets and fractions included). The zero time is year 1,
//! and a value the text format cannot hold (a year outside 0–9999) is an error, not a wrong file.

use crate::util::{civil_date, days_from_civil};
use serde::de::{self, Deserialize, Deserializer};
use serde::ser::{self, Serialize, Serializer};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp {
    secs: i64,
    nanos: u32,
}

const ZERO_SECS: i64 = -62_135_596_800; // 0001-01-01T00:00:00Z
const MAX_SECS: i64 = 253_402_300_799; // 9999-12-31T23:59:59Z

impl Timestamp {
    pub const ZERO: Timestamp = Timestamp { secs: ZERO_SECS, nanos: 0 };

    pub fn from_unix(secs: i64) -> Timestamp {
        Timestamp { secs, nanos: 0 }
    }

    /// 2026-11-20T09:00:00Z, for tests and fixed clocks.
    pub fn ymd_hms(y: i64, m: u32, d: u32, h: i64, mi: i64, s: i64) -> Timestamp {
        Timestamp::from_unix(days_from_civil(y, m, d) * 86_400 + h * 3600 + mi * 60 + s)
    }

    pub fn now() -> Timestamp {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(d) => Timestamp { secs: d.as_secs() as i64, nanos: d.subsec_nanos() },
            Err(_) => Timestamp::ZERO,
        }
    }

    pub fn is_zero(&self) -> bool {
        *self == Timestamp::ZERO
    }

    pub fn unix(&self) -> i64 {
        self.secs
    }

    pub fn add(&self, d: Duration) -> Timestamp {
        let total = self.nanos as u64 + d.subsec_nanos() as u64;
        Timestamp { secs: self.secs.saturating_add(d.as_secs() as i64).saturating_add((total / 1_000_000_000) as i64), nanos: (total % 1_000_000_000) as u32 }
    }

    /// How long after `earlier` this is (zero if it is not later).
    pub fn since(&self, earlier: Timestamp) -> Duration {
        if *self <= earlier {
            return Duration::ZERO;
        }
        let (mut s, mut n) = (self.secs - earlier.secs, self.nanos as i64 - earlier.nanos as i64);
        if n < 0 {
            s -= 1;
            n += 1_000_000_000;
        }
        Duration::new(s as u64, n as u32)
    }

    /// RFC 3339 in UTC with the fraction Go would write (none when zero, trailing zeros trimmed).
    pub fn to_rfc3339(&self) -> Result<String, String> {
        if self.secs < ZERO_SECS || self.secs > MAX_SECS {
            return Err(format!("year outside [0,9999]: {}", self.secs));
        }
        let (y, mo, d) = civil_date(self.secs);
        let t = self.secs.rem_euclid(86_400);
        let mut out = format!("{y:04}-{mo:02}-{d:02}T{:02}:{:02}:{:02}", t / 3600, t % 3600 / 60, t % 60);
        if self.nanos != 0 {
            let frac = format!("{:09}", self.nanos);
            out.push('.');
            out.push_str(frac.trim_end_matches('0'));
        }
        out.push('Z');
        Ok(out)
    }

    pub fn parse(s: &str) -> Result<Timestamp, String> {
        let bad = || format!("{s:?} is not an RFC 3339 time");
        let b = s.as_bytes();
        if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || (b[10] != b'T' && b[10] != b't') || b[13] != b':' || b[16] != b':' {
            return Err(bad());
        }
        let num = |a: usize, z: usize| -> Result<i64, String> { s.get(a..z).filter(|x| x.bytes().all(|c| c.is_ascii_digit())).and_then(|x| x.parse().ok()).ok_or_else(bad) };
        let (y, mo, d, h, mi, sec) = (num(0, 4)?, num(5, 7)?, num(8, 10)?, num(11, 13)?, num(14, 16)?, num(17, 19)?);
        if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 59 {
            return Err(bad());
        }
        let mut i = 19;
        let mut nanos = 0u32;
        if b.get(i) == Some(&b'.') {
            let start = i + 1;
            i = start;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            if i == start || i - start > 9 {
                return Err(bad());
            }
            nanos = format!("{:0<9}", &s[start..i]).parse().map_err(|_| bad())?;
        }
        let offset = match b.get(i) {
            Some(b'Z') | Some(b'z') if i + 1 == b.len() => 0,
            Some(&sign @ (b'+' | b'-')) if i + 6 == b.len() && b[i + 3] == b':' => {
                let (oh, om) = (num(i + 1, i + 3)?, num(i + 4, i + 6)?);
                if oh > 23 || om > 59 {
                    return Err(bad());
                }
                (if sign == b'+' { 1 } else { -1 }) * (oh * 3600 + om * 60)
            }
            _ => return Err(bad()),
        };
        let days = days_from_civil(y, mo as u32, d as u32);
        let (cy, cm, cd) = crate::util::civil_date(days * 86_400);
        if (cy, cm as i64, cd as i64) != (y, mo, d) {
            return Err(bad()); // a day that does not exist (February 30)
        }
        Ok(Timestamp { secs: days * 86_400 + h * 3600 + mi * 60 + sec - offset, nanos })
    }
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_rfc3339().map_err(ser::Error::custom)?)
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Timestamp, D::Error> {
        let s = String::deserialize(d)?;
        Timestamp::parse(&s).map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trips_and_matches_what_go_writes() {
        let t = Timestamp::ymd_hms(2026, 11, 20, 9, 0, 0);
        assert_eq!(t.to_rfc3339().unwrap(), "2026-11-20T09:00:00Z");
        assert_eq!(Timestamp::parse("2026-11-20T09:00:00Z").unwrap(), t);
        assert_eq!(Timestamp::parse("2026-11-20T18:00:00+09:00").unwrap(), t, "a local-time offset, as the Go bridge writes it");
        assert_eq!(Timestamp::parse("2026-11-20T04:30:00-04:30").unwrap(), t);
        let f = Timestamp::parse("2026-11-20T09:00:00.120Z").unwrap();
        assert_eq!(f.to_rfc3339().unwrap(), "2026-11-20T09:00:00.12Z", "trailing zeros of the fraction are trimmed");
        assert_eq!(Timestamp::parse(&f.to_rfc3339().unwrap()).unwrap(), f);
        assert_eq!(Timestamp::ZERO.to_rfc3339().unwrap(), "0001-01-01T00:00:00Z");
        assert!(Timestamp::parse("0001-01-01T00:00:00Z").unwrap().is_zero());
    }

    #[test]
    fn bad_text_is_refused() {
        for s in ["", "yesterday", "2026-11-20", "2026-11-20T09:00:00", "2026-13-20T09:00:00Z", "2026-02-30T09:00:00Z", "2026-11-20T25:00:00Z", "2026-11-20T09:00:00.Z", "2026-11-20T09:00:00+0900", "2026-11-20 09:00:00Z"] {
            assert!(Timestamp::parse(s).is_err(), "{s:?}");
        }
    }

    #[test]
    fn a_year_the_text_cannot_hold_is_an_error() {
        assert!(Timestamp::ymd_hms(10000, 1, 1, 0, 0, 0).to_rfc3339().is_err());
        assert!(serde_json::to_string(&Timestamp::ymd_hms(10000, 1, 1, 0, 0, 0)).is_err());
    }

    #[test]
    fn arithmetic() {
        let t = Timestamp::ymd_hms(2026, 11, 20, 9, 0, 0);
        let u = t.add(Duration::from_millis(1500));
        assert_eq!(u.since(t), Duration::from_millis(1500));
        assert_eq!(t.since(u), Duration::ZERO, "never negative");
        assert!(u > t);
    }
}

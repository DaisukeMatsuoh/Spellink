//! Direct wss:// at home: a publicly trusted certificate for a name that points at the bridge's
//! LAN address (Go: internal/lantls). This file holds the setting and its validation; the
//! certificate acquisition comes with ADR 0017 (the name service).

use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;

/// The user's setting (config.json "lan").
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    /// The DNS name to publish, e.g. "rock.lan.example.com". Its zone must be on Cloudflare.
    #[serde(default)]
    pub host: String,
    /// An API token with Zone → DNS → Edit for that one zone.
    #[serde(default)]
    pub cloudflare_token: String,
    /// Optional (Let's Encrypt expiry notices).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub email: String,
    /// Overrides the zone detected from the host (a two-label guess is wrong for e.g. co.jp).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub zone: String,
    /// Overrides the detected LAN address.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ip: String,
}

/// Whether `s` is a plain DNS name of at least two labels: letters, digits and hyphens, no
/// wildcard, no scheme or port, no trailing dot.
pub fn dns_name(s: &str) -> bool {
    if s.is_empty() || s.len() > 253 || !s.contains('.') {
        return false;
    }
    s.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })
}

/// Whether `ip` is in a private IPv4 range (10/8, 172.16/12, 192.168/16), as Go's `IsPrivate`.
pub fn private_v4(ip: Ipv4Addr) -> bool {
    ip.is_private()
}

impl Config {
    /// What is wrong with the setting, or None when usable. The values end up in public DNS and in
    /// a certificate request, so they are checked, not trusted.
    pub fn valid(&self) -> Option<String> {
        if self.host.is_empty() {
            return Some("lan.host is empty".into());
        }
        if !dns_name(&self.host) {
            return Some(format!("lan.host {:?} is not a plain DNS name like rock.lan.example.com", self.host));
        }
        if self.cloudflare_token.is_empty() {
            return Some("lan.cloudflareToken is empty".into());
        }
        if !self.zone.is_empty() {
            if !dns_name(&self.zone) {
                return Some(format!("lan.zone {:?} is not a DNS name", self.zone));
            }
            let (h, z) = (self.host.to_lowercase(), self.zone.to_lowercase());
            if h != z && !h.ends_with(&format!(".{z}")) {
                return Some(format!("lan.host {:?} is not inside lan.zone {:?}", self.host, self.zone));
            }
        }
        if !self.ip.is_empty() {
            match self.ip.parse::<std::net::IpAddr>() {
                Err(_) => return Some(format!("lan.ip {:?} is not an address", self.ip)),
                // The address is published in public DNS: only a private IPv4 address belongs there.
                Ok(std::net::IpAddr::V4(ip)) if private_v4(ip) => {}
                Ok(_) => return Some(format!("lan.ip {:?} must be a private IPv4 address (it is published in DNS)", self.ip)),
            }
        }
        None
    }
}

//! Decides whether a request may reach the bridge's local-only doors (the command line's admin API,
//! the pairing page, the app page): it must come from this computer's own browser or command line,
//! not through a tunnel or proxy, and not from a web page that tricked the browser into talking to
//! 127.0.0.1 (Go: internal/localguard).

use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use std::net::IpAddr;

/// Whether a request is from the person at this computer:
///   - no proxy or tunnel headers (a tunnel delivers every request from 127.0.0.1; these headers are
///     how it shows), and
///   - the Host is a loopback name. A web page on another site can make a browser send requests to
///     127.0.0.1 after re-pointing its own name there ("DNS rebinding"); the Host it carries is
///     still its own name.
pub fn local(headers: &HeaderMap, host: &str) -> bool {
    for h in ["cf-connecting-ip", "x-forwarded-for", "forwarded"] {
        if headers.get(h).is_some_and(|v| !v.as_bytes().is_empty()) {
            return false;
        }
    }
    loopback_host(host)
}

/// The Host of a request: the Host header (HTTP/1.1) or, failing that, the URI's authority.
pub fn host_of(req: &axum::http::Request<axum::body::Body>) -> String {
    req.headers().get(header::HOST).and_then(|v| v.to_str().ok()).map(String::from).or_else(|| req.uri().authority().map(|a| a.to_string())).unwrap_or_default()
}

fn loopback_host(hostport: &str) -> bool {
    // "host:port", "[v6]:port", "[v6]" or a bare name
    let host = if let Some(rest) = hostport.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        match hostport.rsplit_once(':') {
            Some((h, port)) if !h.contains(':') && port.bytes().all(|b| b.is_ascii_digit()) => h,
            _ => hostport,
        }
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// The answer to a request that is not local.
pub fn refuse() -> Response {
    (StatusCode::FORBIDDEN, "not available here\n").into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(host: &str, headers: &[(&str, &str)]) -> bool {
        let mut h = HeaderMap::new();
        for (k, v) in headers {
            h.insert(axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(), v.parse().unwrap());
        }
        local(&h, host)
    }

    #[test]
    fn loopback_names_are_local() {
        for host in ["127.0.0.1:8765", "127.0.0.1", "localhost:8765", "LOCALHOST", "[::1]:8765", "[::1]", "127.1.2.3:80"] {
            assert!(req(host, &[]), "{host} should be local");
        }
    }

    #[test]
    fn other_names_are_not_local_whatever_they_resolve_to() {
        for host in ["evil.example:8765", "evil.example", "bridge.spellink.uk", "192.168.2.180:8765", "10.0.0.1", "[2001:db8::1]:80", "localhost.evil.example", "127.0.0.1.evil.example", "", "0.0.0.0:8765", "[::]:80"] {
            assert!(!req(host, &[]), "{host:?} must not be local (DNS rebinding)");
        }
    }

    #[test]
    fn a_proxy_or_tunnel_is_never_local() {
        for h in ["Cf-Connecting-Ip", "X-Forwarded-For", "Forwarded"] {
            assert!(!req("127.0.0.1:8765", &[(h, "203.0.113.9")]), "{h} must make a request non-local");
        }
    }
}

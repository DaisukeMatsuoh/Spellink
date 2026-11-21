//! The name service of ADR 0017: it gives each bridge a name under the LAN zone and answers DNS
//! for it, and it writes the TXT records that let the bridge get its own certificate.

pub mod api;
pub mod dns;
pub mod store;

use crate::acme::Publisher;
use crate::edge::Tls;
use crate::lan::Registrar;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

fn data_dir(flags: &HashMap<String, String>) -> Result<PathBuf, String> {
    flags.get("data").filter(|d| !d.is_empty()).map(PathBuf::from).ok_or_else(|| "--data DIR is required (where the identity, the certificate and the records are kept)".to_string())
}

/// What a relay needs to face the internet with its own TLS: its identity, the name service it
/// writes its challenge records through, and the certificate settings. Prints the one record the
/// operator adds to the parent zone (a CNAME for the challenge name).
pub fn service_tls(flags: &HashMap<String, String>, domain: &str, what: &str) -> Result<(Tls, Arc<dyn Registrar>, Arc<dyn Publisher>), String> {
    if crate::names::dns::labels_of(domain).is_none_or(|l| l.len() < 2) {
        return Err(format!("--tls-domain {domain:?} is not a DNS name"));
    }
    let data = data_dir(flags)?;
    let names = flags.get("names").filter(|n| !n.is_empty()).map(String::as_str).unwrap_or(crate::lan::DEFAULT_NAMES);
    let zone = flags.get("zone").filter(|z| !z.is_empty()).map(String::as_str).unwrap_or(crate::lan::DEFAULT_ZONE);
    let identity = Arc::new(crate::namesproto::Identity::load_or_create(&data)?);
    let client = Arc::new(crate::lan::client::NamesClient::new(names, identity.clone(), crate::lan::system_clock())?);
    println!("{what} identity {}", identity.id());
    println!("DNS record to add once, so that the certificate for {domain} can be ordered without any DNS token:");
    println!("  _acme-challenge.{domain}.  CNAME  _acme-challenge.{}.{zone}.", identity.id());
    let tls = Tls { domain: domain.to_string(), data, email: flags.get("email").cloned().unwrap_or_default(), ca: flags.get("ca").cloned().unwrap_or_default() };
    Ok((tls, client.clone(), client))
}

/// `spellink-bridge names`: the name service (ADR 0017).
pub async fn run_cli(flags: &HashMap<String, String>) -> Result<(), String> {
    use tokio_util::sync::CancellationToken;
    let data = data_dir(flags)?;
    let zone_name = flags.get("zone").filter(|z| !z.is_empty()).cloned().unwrap_or_else(|| crate::lan::DEFAULT_ZONE.to_string());
    let ns = flags.get("ns").filter(|n| !n.is_empty()).ok_or("--ns NAME is required: the name of this DNS server, e.g. ns1.spellink.uk (its A record is in the parent zone)")?;
    let hostmaster = flags.get("hostmaster").filter(|h| !h.is_empty()).cloned().unwrap_or_else(|| format!("hostmaster.{}", ns.split_once('.').map(|(_, rest)| rest).unwrap_or(ns)));
    let zone = dns::Zone::new(&zone_name, ns, &hostmaster)?;
    let addr = |flag: &str, default: &str| -> String {
        let v = flags.get(flag).filter(|v| !v.is_empty()).map(String::as_str).unwrap_or(default);
        if let Some(port) = v.strip_prefix(':') { format!("0.0.0.0:{port}") } else { v.to_string() }
    };
    let (dns_addr, api_addr) = (addr("dns", ":53"), addr("listen", ":8443"));
    let log: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(|s| println!("{s}"));
    let clock = crate::lan::system_clock();

    let (names, problems) = store::Names::load(data.join("names.json"), store::Limits::default(), clock())?;
    for p in problems {
        log(&p);
    }
    let names = Arc::new(names);
    let identity = Arc::new(crate::namesproto::Identity::load_or_create(&data)?);
    names.add_local(identity.id(), identity.public_key(), clock());

    let udp = tokio::net::UdpSocket::bind(&dns_addr).await.map_err(|e| format!("DNS {dns_addr} (udp): {e}"))?;
    let tcp = tokio::net::TcpListener::bind(&dns_addr).await.map_err(|e| format!("DNS {dns_addr} (tcp): {e}"))?;
    let api = tokio::net::TcpListener::bind(&api_addr).await.map_err(|e| format!("{api_addr}: {e}"))?;
    let cancel = CancellationToken::new();
    crate::edge::cancel_on_signal(cancel.clone());
    let dns_state = Arc::new(dns::Dns { zone, names: names.clone(), limiter: crate::relay::Limiter::new(100, std::time::Duration::from_secs(1)) });
    tokio::spawn(dns::serve_udp(dns_state.clone(), udp, cancel.clone()));
    tokio::spawn(dns::serve_tcp(dns_state, tcp, cancel.clone()));
    {
        let (names, stop, clock) = (names.clone(), cancel.clone(), clock.clone());
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
            loop {
                tokio::select! { _ = stop.cancelled() => return, _ = tick.tick() => { names.forget_old(clock()); } }
            }
        });
    }

    let tls = match flags.get("tls-domain").filter(|d| !d.is_empty()) {
        None => None,
        Some(domain) => {
            let local = Arc::new(crate::lan::client::LocalNames { names: names.clone(), identity: identity.clone(), clock: clock.clone() });
            println!("names identity {}", identity.id());
            println!("DNS record to add once, so that the certificate for {domain} can be ordered without any DNS token:");
            println!("  _acme-challenge.{domain}.  CNAME  _acme-challenge.{}.{zone_name}.", identity.id());
            let tls = Tls { domain: domain.clone(), data: data.clone(), email: flags.get("email").cloned().unwrap_or_default(), ca: flags.get("ca").cloned().unwrap_or_default() };
            Some((tls, local.clone() as Arc<dyn Registrar>, local as Arc<dyn Publisher>))
        }
    };
    println!("spellink names {} for {zone_name}: DNS on {dns_addr}, API on {api_addr}{}", env!("CARGO_PKG_VERSION"), if tls.is_some() { " (TLS)" } else { "" });
    crate::edge::serve(api, api::router(api::Api::new(names, log.clone())), tls, cancel, log).await
}

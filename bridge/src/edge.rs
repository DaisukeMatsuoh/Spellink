//! How the relay and the name service face the internet (ADR 0017): their own TLS with a
//! certificate they get themselves, or plain HTTP behind something that does TLS (a proxy, a
//! test). The certificate comes the same way the bridge's does: DNS-01, with the TXT record
//! written through the name service (the operator points `_acme-challenge.<name>` at this
//! machine's own challenge name once; docs/guides/relay-operations.md).

use crate::acme::{Acme, CertFiles, Publisher};
use crate::lan::{Addresses, Lan, Parts, Registrar, Timing, system_clock};
use crate::tlsserve::CertResolver;
use axum::Router;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub struct Tls {
    /// The public name of this service, e.g. relay.spellink.uk.
    pub domain: String,
    pub data: PathBuf,
    pub email: String,
    /// "" (Let's Encrypt) or "staging".
    pub ca: String,
}

/// A service has no address to publish: only the TXT record is needed.
struct NoAddress;

impl Addresses for NoAddress {
    fn current(&self) -> Option<String> {
        Some(String::new())
    }
}

/// Serves `router` on `listener` until `cancel`: over TLS when `tls` is given, otherwise plain.
/// `registrar` and `publisher` reach the name service (over the network, or directly when this
/// process is the name service).
pub async fn serve(listener: tokio::net::TcpListener, router: Router, tls: Option<(Tls, Arc<dyn Registrar>, Arc<dyn Publisher>)>, cancel: CancellationToken, log: Arc<dyn Fn(&str) + Send + Sync>) -> Result<(), String> {
    let Some((tls, registrar, publisher)) = tls else {
        let stop = cancel.clone();
        return axum::serve(crate::util::low_latency(listener), router.into_make_service_with_connect_info::<SocketAddr>()).with_graceful_shutdown(async move { stop.cancelled().await }).await.map_err(|e| e.to_string());
    };
    let resolver = CertResolver::new();
    let directory = if tls.ca == "staging" { Acme::staging() } else { Acme::production() };
    let log_for_lan = log.clone();
    let parts = Parts {
        domain: tls.domain.clone(),
        registrar,
        addresses: Arc::new(NoAddress),
        publisher,
        orderer: Arc::new(Acme::new(&tls.data, directory, &tls.email)),
        files: CertFiles::new(&tls.data, "edge"),
        resolver: resolver.clone(),
        clock: system_clock(),
        log: Arc::new(move |s| log_for_lan(s)),
        timing: Timing::default(),
    };
    let lan = Lan::start(parts, cancel.clone());
    let (mut watch, status_log, stop) = (lan.watch(), log.clone(), cancel.clone());
    tokio::spawn(async move {
        let _keep = &lan;
        let mut last = String::new();
        loop {
            let now = watch.borrow_and_update().describe();
            if now != last {
                status_log(&format!("tls {}: {now}", tls.domain));
                last = now;
            }
            tokio::select! {
                _ = stop.cancelled() => return,
                r = watch.changed() => if r.is_err() { return },
            }
        }
    });
    crate::tlsserve::serve(listener, router, resolver, cancel).await
}

/// Cancels `cancel` on SIGINT or SIGTERM.
pub fn cancel_on_signal(cancel: CancellationToken) {
    tokio::spawn(async move {
        crate::app::wait_for_signal().await;
        cancel.cancel();
    });
}

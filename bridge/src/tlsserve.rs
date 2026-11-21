//! A TLS front for an axum router, with a certificate that can be replaced while it runs (the
//! bridge's LAN certificate, the relay's, the name service's). Without a valid certificate the
//! handshake is refused: an expired certificate is never presented (docs/spec/lan-cert.md L2).

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use hyper::body::Incoming;
use hyper::{Request, service::service_fn};
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

/// How long a client may take to finish the TLS handshake, and to send the HTTP request line and
/// headers. A connection that is slower is closed (slow-connection attacks).
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
/// How many handshakes run at once (they cost CPU) and how many connections are open at most.
const MAX_HANDSHAKES: usize = 128;
const MAX_CONNECTIONS: usize = 8192;

#[derive(Default)]
pub struct CertResolver {
    current: RwLock<Option<Arc<CertifiedKey>>>,
}

impl std::fmt::Debug for CertResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CertResolver").field("has_certificate", &self.has()).finish()
    }
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::aws_lc_rs::default_provider())
}

impl CertResolver {
    pub fn new() -> Arc<CertResolver> {
        Arc::new(CertResolver::default())
    }

    /// Starts presenting this certificate. A chain and key that do not belong together are refused
    /// and the old certificate stays.
    pub fn set(&self, chain_pem: &str, key_pem: &str) -> Result<(), String> {
        let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(chain_pem.as_bytes()).collect::<Result<_, _>>().map_err(|e| format!("certificate chain: {e}"))?;
        if chain.is_empty() {
            return Err("certificate chain: empty".into());
        }
        let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes()).map_err(|e| format!("private key: {e}"))?;
        let certified = CertifiedKey::from_der(chain, key, &provider()).map_err(|e| format!("certificate and key: {e}"))?;
        *self.current.write().unwrap() = Some(Arc::new(certified));
        Ok(())
    }

    /// Stops presenting a certificate: handshakes are refused until `set`.
    pub fn clear(&self) {
        *self.current.write().unwrap() = None;
    }

    pub fn has(&self) -> bool {
        self.current.read().unwrap().is_some()
    }
}

impl ResolvesServerCert for CertResolver {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        self.current.read().unwrap().clone()
    }
}

pub fn server_config(resolver: Arc<CertResolver>) -> Result<Arc<rustls::ServerConfig>, String> {
    let mut cfg = rustls::ServerConfig::builder_with_provider(provider()).with_safe_default_protocol_versions().map_err(|e| e.to_string())?.with_no_client_auth().with_cert_resolver(resolver);
    // WebSocket upgrades are HTTP/1.1
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(cfg))
}

/// Serves `app` over TLS on `listener` until `cancel`. The peer's address is given to handlers
/// the same way `into_make_service_with_connect_info` does.
pub async fn serve(listener: tokio::net::TcpListener, app: Router, resolver: Arc<CertResolver>, cancel: CancellationToken) -> Result<(), String> {
    let acceptor = tokio_rustls::TlsAcceptor::from(server_config(resolver)?);
    let handshakes = Arc::new(tokio::sync::Semaphore::new(MAX_HANDSHAKES));
    let connections = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
    loop {
        let (tcp, peer) = tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            r = listener.accept() => match r { Ok(v) => v, Err(_) => { tokio::time::sleep(Duration::from_millis(50)).await; continue } },
        };
        // a full house: close at once rather than queue
        let Ok(conn_slot) = connections.clone().try_acquire_owned() else { continue };
        let _ = tcp.set_nodelay(true); // best effort; a refusing socket still works
        let (acceptor, handshakes, app, stop) = (acceptor.clone(), handshakes.clone(), app.clone(), cancel.clone());
        tokio::spawn(async move {
            let _conn_slot = conn_slot;
            let tls = {
                let Ok(_hs) = handshakes.try_acquire() else { return };
                match tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(tcp)).await {
                    Ok(Ok(t)) => t,
                    _ => return, // refused, failed or too slow
                }
            };
            let service = service_fn(move |mut req: Request<Incoming>| {
                let app = app.clone();
                req.extensions_mut().insert(ConnectInfo::<SocketAddr>(peer));
                async move { app.oneshot(req.map(Body::new)).await }
            });
            let mut builder = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());
            builder.http1().timer(TokioTimer::new()).header_read_timeout(HEADER_TIMEOUT);
            let conn = builder.serve_connection_with_upgrades(TokioIo::new(tls), service);
            tokio::pin!(conn);
            tokio::select! {
                _ = &mut conn => {}
                _ = stop.cancelled() => {}
            }
        });
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A self-signed certificate for `name`: (chain pem, key pem).
    pub fn self_signed(name: &str) -> (String, String) {
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = rcgen::CertificateParams::new(vec![name.to_string()]).unwrap().self_signed(&key).unwrap();
        (cert.pem(), key.serialize_pem())
    }

    pub fn client_for(chain_pem: &str) -> tokio_rustls::TlsConnector {
        let mut roots = rustls::RootCertStore::empty();
        for c in CertificateDer::pem_slice_iter(chain_pem.as_bytes()) {
            roots.add(c.unwrap()).unwrap();
        }
        let cfg = rustls::ClientConfig::builder_with_provider(provider()).with_safe_default_protocol_versions().unwrap().with_root_certificates(roots).with_no_client_auth();
        tokio_rustls::TlsConnector::from(Arc::new(cfg))
    }

    async fn get(addr: SocketAddr, connector: &tokio_rustls::TlsConnector, name: &str) -> Result<String, String> {
        let tcp = tokio::net::TcpStream::connect(addr).await.map_err(|e| e.to_string())?;
        let mut tls = connector.connect(rustls::pki_types::ServerName::try_from(name.to_string()).unwrap(), tcp).await.map_err(|e| e.to_string())?;
        tls.write_all(b"GET /who HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").await.map_err(|e| e.to_string())?;
        let mut out = String::new();
        let _ = tls.read_to_string(&mut out).await;
        Ok(out)
    }

    #[tokio::test]
    async fn serves_over_tls_with_the_peer_address_and_swaps_the_certificate_without_a_restart() {
        let (chain1, key1) = self_signed("one.test");
        let (chain2, key2) = self_signed("two.test");
        let resolver = CertResolver::new();
        let app = Router::new().route("/who", axum::routing::get(|ConnectInfo(p): ConnectInfo<SocketAddr>| async move { format!("peer={}", p.ip()) }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let cancel = CancellationToken::new();
        tokio::spawn(serve(listener, app, resolver.clone(), cancel.clone()));

        // no certificate yet: the handshake is refused
        assert!(get(addr, &client_for(&chain1), "one.test").await.is_err(), "nothing is presented without a certificate");

        resolver.set(&chain1, &key1).unwrap();
        let body = get(addr, &client_for(&chain1), "one.test").await.unwrap();
        assert!(body.starts_with("HTTP/1.1 200") && body.ends_with("peer=127.0.0.1"), "{body}");

        // the new certificate is used by the next connection; the old name no longer matches
        resolver.set(&chain2, &key2).unwrap();
        assert!(get(addr, &client_for(&chain2), "two.test").await.unwrap().contains("peer="));
        assert!(get(addr, &client_for(&chain1), "one.test").await.is_err());

        // clearing stops presenting it again (an expired certificate must not be shown)
        resolver.clear();
        assert!(get(addr, &client_for(&chain2), "two.test").await.is_err());
        cancel.cancel();
    }

    #[test]
    fn a_key_that_does_not_belong_to_the_chain_is_refused_and_the_old_certificate_stays() {
        let (chain1, key1) = self_signed("one.test");
        let (_chain2, key2) = self_signed("two.test");
        let r = CertResolver::new();
        r.set(&chain1, &key1).unwrap();
        assert!(r.set(&chain1, &key2).is_err(), "someone else's key");
        assert!(r.set("", &key1).is_err() && r.set(&chain1, "").is_err() && r.set("garbage", "garbage").is_err());
        assert!(r.has(), "still presenting the first one");
    }

    #[tokio::test]
    async fn a_connection_that_never_speaks_is_closed() {
        tokio::time::pause();
        let (chain, key) = self_signed("one.test");
        let resolver = CertResolver::new();
        resolver.set(&chain, &key).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let cancel = CancellationToken::new();
        tokio::spawn(serve(listener, Router::new(), resolver, cancel.clone()));
        let mut idle = tokio::net::TcpStream::connect(addr).await.unwrap();
        tokio::time::sleep(HANDSHAKE_TIMEOUT + Duration::from_secs(1)).await;
        let mut buf = [0u8; 8];
        let n = tokio::time::timeout(Duration::from_secs(1), idle.read(&mut buf)).await;
        assert!(matches!(n, Ok(Ok(0)) | Ok(Err(_))), "the server closed the silent connection: {n:?}");
        cancel.cancel();
    }
}

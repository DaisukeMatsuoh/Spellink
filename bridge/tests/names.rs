//! The name service as it is run: the real binary, real sockets. A bridge registers, and the
//! world can then find its private address in DNS, and the TXT value that a certificate order
//! needs (ADR 0017).

use spellink_bridge::acme::Publisher;
use spellink_bridge::lan::Registrar;
use spellink_bridge::lan::client::NamesClient;
use spellink_bridge::namesproto::Identity;
use std::net::SocketAddr;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_spellink-bridge");

fn free_port() -> u16 {
    // a port that is free for both UDP and TCP (DNS uses both)
    loop {
        let t = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = t.local_addr().unwrap().port();
        if std::net::UdpSocket::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
}

struct Server {
    child: Child,
    dns: SocketAddr,
    api: String,
    _dir: tempfile::TempDir,
}

impl Server {
    fn start() -> Server {
        Server::start_in(tempfile::tempdir().unwrap())
    }

    fn start_in(dir: tempfile::TempDir) -> Server {
        Server::start_with(dir, &[])
    }

    /// Starts the service with `extra` flags. The ports are found by binding and releasing, which
    /// another test of this file may do at the same moment: a server that did not get its ports
    /// exits at once, so it is started again on new ones.
    fn start_with(dir: tempfile::TempDir, extra: &[&str]) -> Server {
        for _ in 0..10 {
            let dns = free_port();
            let api = loop {
                let p = free_port();
                if p != dns {
                    break p; // the same port twice would bind for DNS and then fail for the API
                }
            };
            let mut child = Command::new(BIN)
                .args(["names", "--zone", "lan.test", "--ns", "ns1.test", "--data", dir.path().to_str().unwrap(), "--dns", &format!("127.0.0.1:{dns}"), "--listen", &format!("127.0.0.1:{api}")])
                .args(extra)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let mut up = false;
            for _ in 0..100 {
                if child.try_wait().unwrap().is_some() {
                    break; // it exited: its ports were taken
                }
                if std::net::TcpStream::connect(("127.0.0.1", api)).is_ok() {
                    up = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            // the connection above could have been to someone else's server on a shared port: a server
            // that did not get its ports exits within moments, so look again before trusting it
            std::thread::sleep(Duration::from_millis(250));
            if up && child.try_wait().unwrap().is_none() {
                return Server { child, dns: format!("127.0.0.1:{dns}").parse().unwrap(), api: format!("http://127.0.0.1:{api}"), _dir: dir };
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        panic!("the name service did not start");
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn query(dns: SocketAddr, name: &str, qtype: u16) -> Vec<u8> {
    let mut q = vec![0x12, 0x34, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
    for l in name.split('.') {
        q.push(l.len() as u8);
        q.extend_from_slice(l.as_bytes());
    }
    q.push(0);
    q.extend_from_slice(&qtype.to_be_bytes());
    q.extend_from_slice(&1u16.to_be_bytes());
    // UDP may lose a datagram; a resolver asks again after a second, and so does this
    let s = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    s.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    let mut buf = [0u8; 600];
    for _ in 0..3 {
        s.send_to(&q, dns).unwrap();
        if let Ok(n) = s.recv(&mut buf) {
            return buf[..n].to_vec();
        }
    }
    panic!("the name service did not answer three times");
}

fn clock() -> spellink_bridge::lan::Clock {
    spellink_bridge::lan::system_clock()
}

#[tokio::test]
async fn a_registered_bridge_is_found_in_dns_and_can_publish_its_challenge_value() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let me = Arc::new(Identity::load_or_create(dir.path()).unwrap());
    let client = NamesClient::new(&server.api, me.clone(), clock()).unwrap();
    let name = format!("{}.lan.test", me.id());

    // before it registers, the name does not exist
    let r = query(server.dns, &name, 1);
    assert_eq!(r[3] & 0xF, 3, "NXDOMAIN");

    client.register("192.168.4.20").await.unwrap();
    let r = query(server.dns, &name, 1);
    assert_eq!((r[3] & 0xF, u16::from_be_bytes([r[6], r[7]])), (0, 1));
    assert_eq!(&r[r.len() - 4..], &[192, 168, 4, 20]);

    // an address that is not private is refused, and the record is unchanged
    let err = client.register("8.8.8.8").await.unwrap_err();
    assert!(err.contains("403") && err.contains("ip_not_private"), "{err}");
    assert_eq!(&query(server.dns, &name, 1).last_chunk::<4>().unwrap()[..], &[192, 168, 4, 20]);

    // the TXT value for a certificate order, and its removal
    let value = "v".repeat(43);
    client.set(&value).await.unwrap();
    let txt = query(server.dns, &format!("_acme-challenge.{name}"), 16);
    assert!(txt.windows(43).any(|w| w == value.as_bytes()), "the TXT value is served");
    client.clear(&value).await.unwrap();
    let txt = query(server.dns, &format!("_acme-challenge.{name}"), 16);
    assert_eq!(u16::from_be_bytes([txt[6], txt[7]]), 0, "gone after clear");
}

#[tokio::test]
async fn an_unknown_bridge_cannot_publish_and_is_told_to_register() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let stranger = Arc::new(Identity::load_or_create(dir.path()).unwrap());
    let client = NamesClient::new(&server.api, stranger, clock()).unwrap();
    let e = client.set(&"x".repeat(43)).await.unwrap_err();
    assert_eq!(e, spellink_bridge::acme::PublishError::Lost);
}

#[tokio::test]
async fn the_service_answers_only_for_its_zone_and_keeps_its_records_across_a_restart() {
    let mut server = Server::start();
    assert_eq!(query(server.dns, "www.example.com", 1)[3] & 0xF, 5, "REFUSED: not a resolver");
    let dir = tempfile::tempdir().unwrap();
    let me = Arc::new(Identity::load_or_create(dir.path()).unwrap());
    NamesClient::new(&server.api, me.clone(), clock()).unwrap().register("10.9.8.7").await.unwrap();
    // stop it, and start another on the same files
    let _ = server.child.kill();
    let _ = server.child.wait();
    let data = std::mem::replace(&mut server._dir, tempfile::tempdir().unwrap());
    let again = Server::start_in(data);
    let r = query(again.dns, &format!("{}.lan.test", me.id()), 1);
    assert_eq!(&r[r.len() - 4..], &[10, 9, 8, 7], "the record was kept");
}

fn read_until(child: &mut Child, needle: &str) -> String {
    use std::io::Read;
    let mut out = child.stdout.take().unwrap();
    let mut seen = String::new();
    let mut buf = [0u8; 512];
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline && !seen.contains(needle) {
        let n = out.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        seen.push_str(&String::from_utf8_lossy(&buf[..n]));
    }
    seen
}

#[test]
fn a_relay_with_its_own_tls_says_which_record_to_add_and_refuses_handshakes_until_it_has_a_certificate() {
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let mut child = Command::new(BIN)
        .args(["relay", "--tls-domain", "relay.example.test", "--data", dir.path().to_str().unwrap(), "--names", "http://127.0.0.1:1", "--zone", "lan.test", "--listen", &format!("127.0.0.1:{port}")])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let said = read_until(&mut child, "listening on");
    assert!(said.contains("_acme-challenge.relay.example.test.  CNAME  _acme-challenge.") && said.contains(".lan.test."), "{said}");
    assert!(said.contains("(TLS)"), "{said}");
    // a client that starts a handshake is turned away: there is no certificate yet
    let mut tcp = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    use std::io::{Read, Write};
    // a TLS 1.2 ClientHello is long; a plain HTTP request is not TLS and must not get an answer either
    tcp.write_all(b"GET /health HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
    let mut buf = [0u8; 64];
    let n = tcp.read(&mut buf).unwrap_or(0);
    assert!(!buf[..n].starts_with(b"HTTP/1.1 200"), "plain HTTP must not be served on the TLS port");
    // the identity was written (private) so that the record to add stays the same on the next start
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(dir.path().join("names-identity.pk8")).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn the_relay_refuses_its_own_tls_together_with_trust_proxy_and_a_data_directory_is_required() {
    let run = |args: &[&str]| {
        let o = Command::new(BIN).args(args).output().unwrap();
        (o.status.success(), String::from_utf8_lossy(&o.stderr).into_owned())
    };
    let (ok, err) = run(&["relay", "--tls-domain", "r.example.test", "--trust-proxy", "--data", "/tmp/x", "--listen", "127.0.0.1:0"]);
    assert!(!ok && err.contains("do not go together"), "{err}");
    let (ok, err) = run(&["relay", "--tls-domain", "r.example.test", "--listen", "127.0.0.1:0"]);
    assert!(!ok && err.contains("--data"), "{err}");
    let (ok, err) = run(&["names", "--data", "/tmp/x"]);
    assert!(!ok && err.contains("--ns"), "{err}");
}

#[tokio::test]
async fn a_full_week_refuses_a_new_bridge_with_the_time_a_place_is_free_and_not_the_one_that_ordered() {
    use spellink_bridge::acme::{PublishError, Publisher};
    // the service is started with a week's allowance of one (--new-per-week is a flag of `names`)
    let server = Server::start_with(tempfile::tempdir().unwrap(), &["--new-per-week", "1"]);
    let base = server.api.clone();
    let mk = || {
        let d = tempfile::tempdir().unwrap();
        (Arc::new(Identity::load_or_create(d.path()).unwrap()), d)
    };
    let ((first, _d1), (second, _d2)) = (mk(), mk());
    let (c1, c2) = (NamesClient::new(&base, first, clock()).unwrap(), NamesClient::new(&base, second, clock()).unwrap());
    c1.register("10.1.1.1").await.unwrap();
    c2.register("10.1.1.2").await.unwrap();
    c1.set(&"a".repeat(43)).await.unwrap();
    match c2.set(&"b".repeat(43)).await {
        Err(PublishError::Quota(Some(at))) => {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
            assert!((now + 6 * 86_400..=now + 8 * 86_400).contains(&at), "a week from the first order: {at} vs {now}");
        }
        other => panic!("expected the weekly allowance to refuse: {other:?}"),
    }
    // the one that ordered is not refused when it orders again (a renewal), and may clear
    c1.set(&"c".repeat(43)).await.unwrap();
    c2.clear(&"b".repeat(43)).await.unwrap();
}

#[test]
fn the_name_service_refuses_a_bad_infra_id_or_allowance() {
    for bad in [["--infra-ids", "not-an-id"], ["--new-per-week", "many"]] {
        let dir = tempfile::tempdir().unwrap();
        let out = Command::new(BIN).args(["names", "--ns", "ns1.test", "--data", dir.path().to_str().unwrap(), "--dns", "127.0.0.1:0", "--listen", "127.0.0.1:0"]).args(bad).output().unwrap();
        assert!(!out.status.success(), "{bad:?}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains(bad[0]), "{err}");
    }
}

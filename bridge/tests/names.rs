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
        let (dns, api) = (free_port(), free_port());
        let child = Command::new(BIN)
            .args(["names", "--zone", "lan.test", "--ns", "ns1.test", "--data", dir.path().to_str().unwrap(), "--dns", &format!("127.0.0.1:{dns}"), "--listen", &format!("127.0.0.1:{api}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let s = Server { child, dns: format!("127.0.0.1:{dns}").parse().unwrap(), api: format!("http://127.0.0.1:{api}"), _dir: dir };
        for _ in 0..100 {
            if std::net::TcpStream::connect(("127.0.0.1", api)).is_ok() {
                return s;
            }
            std::thread::sleep(Duration::from_millis(50));
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
    let s = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    s.send_to(&q, dns).unwrap();
    let mut buf = [0u8; 600];
    let n = s.recv(&mut buf).expect("the name service answered");
    buf[..n].to_vec()
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

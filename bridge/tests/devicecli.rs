//! `spellink-bridge device …` against a real bridge's admin door on a real ledger (Go:
//! devicecli_test.go).

mod common;

use common::*;
use spellink_bridge::devicecli::{self, Client};
use spellink_bridge::devices::registry::Conn;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const TOKEN: &str = ADMIN_TOKEN;

struct Env {
    p: PairEnv,
    c: Client,
    dir: tempfile::TempDir,
}

struct StubConn;
impl Conn for StubConn {
    fn grant(&self, _: &str, _: &[String]) -> Result<(), String> {
        Ok(())
    }
    fn deny(&self, _: &str) {}
}

async fn env() -> Env {
    let p = PairEnv::with(|_, a| {
        a.as_mut().unwrap().package = Arc::new(|d| Ok(format!("PACKAGE:{}:{}", d.name, d.b).into_bytes()));
    })
    .await;
    let c = Client { base: format!("http://{}", p.http), token: TOKEN.into(), open: None };
    Env { p, c, dir: tempfile::tempdir().unwrap() }
}

impl Env {
    async fn run(&self, args: &[&str]) -> (String, Result<(), String>) {
        self.run_with(None, args).await
    }

    async fn run_with(&self, stdin: Option<&str>, args: &[&str]) -> (String, Result<(), String>) {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let mut lines: Vec<String> = stdin.map(|s| s.split_inclusive('\n').map(String::from).collect()).unwrap_or_default();
        lines.reverse();
        let mut out = Vec::new();
        let r = devicecli::run(&self.c, &args, &mut || lines.pop(), &mut out, self.dir.path()).await;
        (String::from_utf8(out).unwrap(), r)
    }

    async fn must_run(&self, args: &[&str]) -> String {
        let (out, r) = self.run(args).await;
        r.unwrap_or_else(|e| panic!("{args:?}: {e}"));
        out
    }

    fn file(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }
}

#[tokio::test]
async fn add_writes_a_private_setup_file_and_says_what_to_do_next() {
    let e = env().await;
    let out = e.must_run(&["add", "Nomad"]).await;
    let path = e.file("Spellink-Nomad.snplg");
    let data = std::fs::read_to_string(&path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }
    assert!(data.starts_with("PACKAGE:Nomad:"), "content {data:?}");
    for want in [path.to_str().unwrap(), "Copy the file", "tap Pair", "device pair", "Keep it private"] {
        assert!(out.contains(want), "output lacks {want:?}:\n{out}");
    }
    let other = tempfile::tempdir().unwrap();
    e.must_run(&["add", "Manta", other.path().to_str().unwrap()]).await;
    assert!(other.path().join("Spellink-Manta.snplg").exists());
}

#[tokio::test]
async fn add_refuses_what_it_cannot_do() {
    let e = env().await;
    e.must_run(&["add", "Nomad"]).await;
    assert!(e.run(&["add", "Nomad"]).await.1.unwrap_err().contains("already exists"), "duplicate");
    assert!(e.run(&["add", ""]).await.1.is_err(), "an empty name");
    assert!(e.run(&["add"]).await.1.unwrap_err().contains("usage"), "no name");
    // two names that make the same file name: the second device is added, its file is not written over the first
    e.must_run(&["add", "A B"]).await;
    let err = e.run(&["add", "A_B"]).await.1.unwrap_err();
    assert!(err.contains("device package"), "clash: {err}");
    assert!(std::fs::read_to_string(e.file("Spellink-A_B.snplg")).unwrap().contains(":A B:"), "the first file was overwritten");
    assert!(e.run(&["add", "Third", e.file("missing").to_str().unwrap()]).await.1.is_err(), "missing directory");
}

#[tokio::test]
async fn list_shows_each_state_without_secrets() {
    let e = env().await;
    assert!(e.must_run(&["list"]).await.contains("No devices yet"));
    e.must_run(&["add", "issued"]).await;
    let waiting = e.p.reg.add("waiting").unwrap();
    e.p.reg.request(&waiting, 1, "11111111", Arc::new(StubConn)).unwrap();
    let active = e.p.reg.add("active").unwrap();
    e.p.reg.request(&active, 1, "22222222", Arc::new(StubConn)).unwrap();
    e.p.reg.confirm("22222222").unwrap();
    e.p.reg.ack(&active, 1).unwrap();
    e.p.reg.connect_k(&active).unwrap();
    let revoked = e.p.reg.add("revoked").unwrap();
    e.p.reg.revoke(&revoked).unwrap();
    let out = e.must_run(&["list"]).await;
    for want in ["issued", "waiting", "active", "revoked", "1 waiting for a code", "last seen", "setup file valid until", "NAME", "STATE"] {
        assert!(out.contains(want), "list lacks {want:?}:\n{out}");
    }
    for d in e.p.reg.list() {
        let b = e.p.reg.repack(&d.id).map(|p| p.b).unwrap_or_default();
        assert!(b.is_empty() || !out.contains(&b), "a secret was printed");
    }
    let header = out.lines().next().unwrap();
    assert!(header.starts_with("NAME") && header.contains("  ID"), "{header}");
}

#[tokio::test]
async fn pair_with_the_code_on_the_command_line_or_asked_for() {
    let e = env().await;
    let id = e.p.reg.add("Nomad").unwrap();
    e.p.reg.request(&id, 1, "12345678", Arc::new(StubConn)).unwrap();
    assert!(e.must_run(&["pair", "12345678"]).await.contains("\"Nomad\" is being paired"));
    let id2 = e.p.reg.add("Manta").unwrap();
    e.p.reg.request(&id2, 1, "87654321", Arc::new(StubConn)).unwrap();
    let (out, r) = e.run_with(Some("87654321\n"), &["pair"]).await;
    assert!(r.is_ok() && out.contains("Code shown on the device") && out.contains("\"Manta\""), "asked: {out:?} {r:?}");
}

#[tokio::test]
async fn pair_explains_each_way_it_can_fail() {
    let e = env().await;
    assert!(e.run(&["pair", "12345678"]).await.1.unwrap_err().contains("no device is showing a code"), "nobody waiting");
    let id = e.p.reg.add("Nomad").unwrap();
    e.p.reg.request(&id, 1, "12345678", Arc::new(StubConn)).unwrap();
    let err = e.run(&["pair", "00000000"]).await.1.unwrap_err();
    assert!(err.contains("not accepted") && !err.contains("Nomad"), "wrong code: {err}");
    assert!(e.run(&["pair", "12"]).await.1.unwrap_err().contains("eight digits"), "short code");
    assert!(e.run_with(Some(""), &["pair"]).await.1.unwrap_err().contains("no code"), "no input");
    assert!(e.run_with(Some("12345678"), &["pair"]).await.1.is_ok(), "the last line may have no newline");
}

#[tokio::test]
async fn package_writes_the_file_again_only_once() {
    let e = env().await;
    let id = e.p.reg.add("Nomad").unwrap();
    assert!(e.must_run(&["package", "Nomad"]).await.contains("Spellink-Nomad.snplg"));
    assert!(e.run(&["package", "Nomad"]).await.1.unwrap_err().contains("remove it first"), "second time");
    let other = tempfile::tempdir().unwrap();
    assert!(e.run(&["package", &id, other.path().to_str().unwrap()]).await.1.is_ok(), "by id, into another directory");
    assert!(e.run(&["package", "nobody"]).await.1.unwrap_err().contains("no device named"));
    assert!(e.run(&["package"]).await.1.is_err(), "usage");
    e.p.reg.revoke(&id).unwrap();
    let third = tempfile::tempdir().unwrap();
    assert!(e.run(&["package", "Nomad", third.path().to_str().unwrap()]).await.1.unwrap_err().contains("revoked"), "a revoked device has no package");
}

#[tokio::test]
async fn revoke_reissue_forget() {
    let e = env().await;
    e.p.reg.add("Nomad").unwrap();
    assert!(e.must_run(&["reissue", "Nomad"]).await.contains("pair it again"));
    assert!(e.must_run(&["revoke", "Nomad"]).await.contains("cannot connect any more"));
    assert!(e.run(&["revoke", "Nomad"]).await.1.unwrap_err().contains("already revoked"), "twice");
    assert!(e.must_run(&["forget", "Nomad"]).await.contains("Forgot"));
    assert!(e.run(&["forget", "Nomad"]).await.1.is_err(), "forgetting twice");
    for c in ["revoke", "reissue", "forget"] {
        assert!(e.run(&[c]).await.1.unwrap_err().contains("usage"), "{c} without a name");
    }
}

#[tokio::test]
async fn usage() {
    let e = env().await;
    for args in [vec![], vec!["frobnicate"]] {
        assert!(e.run(&args).await.1.unwrap_err().contains("usage: spellink-bridge device"), "{args:?}");
    }
}

#[tokio::test]
async fn the_bridge_being_down_or_refusing_the_token_is_explained() {
    let down = Client { base: "http://127.0.0.1:1".into(), token: TOKEN.into(), open: None };
    let mut out = Vec::new();
    let r = devicecli::run(&down, &["list".to_string()], &mut || None, &mut out, std::path::Path::new(".")).await;
    assert!(r.unwrap_err().contains("is it running"), "down");
    let mut e = env().await;
    e.c.token = "wrong".into();
    assert!(e.run(&["list"]).await.1.unwrap_err().contains("admin token"), "wrong token");
    let bad = Client { base: "http://[::1".into(), token: TOKEN.into(), open: None };
    assert!(devicecli::run(&bad, &["list".to_string()], &mut || None, &mut Vec::new(), std::path::Path::new(".")).await.is_err(), "an unusable address");
}

/// A server that answers every request with the same bytes.
async fn fake_server(answer: &'static [u8]) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = listener.accept().await else { return };
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf).await;
            let _ = s.write_all(answer).await;
            let _ = s.shutdown().await;
        }
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn an_answer_it_cannot_read_is_reported_not_trusted() {
    let garbage = fake_server(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\nnot json").await;
    let c = Client { base: garbage, token: TOKEN.into(), open: None };
    let dir = tempfile::tempdir().unwrap();
    for args in [vec!["list"], vec!["add", "x"], vec!["pair", "12345678"], vec!["revoke", "x"]] {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        assert!(devicecli::run(&c, &args, &mut || None, &mut Vec::new(), dir.path()).await.is_err(), "{args:?}: an unreadable answer was accepted");
    }
}

#[tokio::test]
async fn an_answer_that_is_cut_off_is_an_error() {
    let cut = fake_server(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nshort").await;
    let c = Client { base: cut, token: TOKEN.into(), open: None };
    assert!(devicecli::run(&c, &["list".to_string()], &mut || None, &mut Vec::new(), std::path::Path::new(".")).await.is_err(), "a truncated answer must not be taken as complete");
}

#[tokio::test]
async fn pair_web_opens_the_page_with_the_token_after_the_hash() {
    let mut e = env().await;
    let opened = Arc::new(Mutex::new(String::new()));
    let o2 = opened.clone();
    e.c.open = Some(Arc::new(move |url| {
        *o2.lock().unwrap() = url.to_string();
        Ok(())
    }));
    let out = e.must_run(&["pair", "--web"]).await;
    assert_eq!(*opened.lock().unwrap(), format!("{}/pair#{TOKEN}", e.c.base));
    assert!(out.contains("Opened the pairing page"));
    assert!(!out.contains(TOKEN), "when the browser opens, the token is not printed");
}

#[tokio::test]
async fn pair_web_prints_the_address_when_the_browser_cannot_be_opened() {
    let mut e = env().await;
    let openers: Vec<(&str, Option<devicecli::Opener>)> = vec![("no opener", None), ("a failing opener", Some(Arc::new(|_| Err("no browser".to_string()))))];
    for (name, open) in openers {
        e.c.open = open;
        let out = e.must_run(&["pair", "--web"]).await;
        assert!(out.contains(&format!("{}/pair#{TOKEN}", e.c.base)) && out.contains("Open this address"), "{name}: {out:?}");
    }
}

#[tokio::test]
async fn pair_web_says_so_when_the_bridge_is_not_running() {
    let c = Client { base: "http://127.0.0.1:1".into(), token: TOKEN.into(), open: Some(Arc::new(|_| panic!("a browser was opened for a bridge that is not there"))) };
    let r = devicecli::run(&c, &["pair".into(), "--web".into()], &mut || None, &mut Vec::new(), std::path::Path::new(".")).await;
    assert!(r.unwrap_err().contains("is it running"));
}

#[tokio::test]
async fn open_app_opens_the_app_page_with_the_token_after_the_hash() {
    let mut e = env().await;
    let opened = Arc::new(Mutex::new(String::new()));
    let o2 = opened.clone();
    e.c.open = Some(Arc::new(move |url| {
        *o2.lock().unwrap() = url.to_string();
        Ok(())
    }));
    let mut out = Vec::new();
    devicecli::open_app(&e.c, &mut out).await.unwrap();
    let said = String::from_utf8(out).unwrap();
    assert_eq!(*opened.lock().unwrap(), format!("{}/app#{TOKEN}", e.c.base));
    assert!(said.contains("Spellink Bridge App") && !said.contains(TOKEN));
    e.c.open = None;
    let mut out = Vec::new();
    devicecli::open_app(&e.c, &mut out).await.unwrap();
    assert!(String::from_utf8(out).unwrap().contains(&format!("{}/app#{TOKEN}", e.c.base)), "without a browser");
    let down = Client { base: "http://127.0.0.1:1".into(), token: TOKEN.into(), open: None };
    assert!(devicecli::open_app(&down, &mut Vec::new()).await.unwrap_err().contains("is it running"));
}

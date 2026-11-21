//! Updating the bridge means replacing its file and starting it again. Nothing the user has — the
//! token (every paired device depends on it), the settings, the devices, the conversations, the
//! reports — may be lost or changed by that, including settings this version does not know (Go:
//! upgrade_test.go). The real binary, the real files.

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_spellink-bridge");
const TOKEN: &str = "the-token-every-device-depends-on";

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                out.insert(p.strip_prefix(root).unwrap().to_string_lossy().into_owned(), std::fs::read(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

struct Bridge {
    child: Child,
}

impl Bridge {
    fn start(exe: &Path, tmp: &Path, port: u16) -> Bridge {
        let log = std::fs::File::create(tmp.join("serve.log")).unwrap();
        let child = Command::new(exe)
            .arg("serve")
            .env("HOME", tmp)
            .env("XDG_CONFIG_HOME", tmp.join("cfg"))
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log))
            .spawn()
            .unwrap();
        let mut b = Bridge { child };
        for i in 0.. {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                break;
            }
            assert!(i < 100, "the bridge did not start:\n{}", std::fs::read_to_string(tmp.join("serve.log")).unwrap_or_default());
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = &mut b;
        b
    }

    /// Stops it with SIGTERM, as the service manager does; it must stop by itself.
    fn stop(mut self) {
        let _ = Command::new("kill").arg("-TERM").arg(self.child.id().to_string()).status();
        for _ in 0..100 {
            if self.child.try_wait().unwrap().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.child.kill();
        panic!("the bridge did not stop on SIGTERM");
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn run(args: &[&str], tmp: &Path, exe: &Path) -> String {
    let o = Command::new(exe).args(args).env("HOME", tmp).env("XDG_CONFIG_HOME", tmp.join("cfg")).output().unwrap();
    assert!(o.status.success(), "{args:?}: {}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn overwriting_the_bridge_keeps_everything_the_user_has() {
    let tmp = tempfile::tempdir().unwrap();
    let tmp = tmp.path();
    let exe: PathBuf = tmp.join("bin/spellink-bridge");
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    let cfg_dir = tmp.join("cfg/spellink-bridge");
    std::fs::create_dir_all(cfg_dir.join("history")).unwrap();
    std::fs::copy(BIN, &exe).unwrap();
    run(&["testfont", cfg_dir.join("glyphs.bin").to_str().unwrap()], tmp, &exe);

    let port = free_port();
    let config = json!({
        "name": "upgrade", "port": port, "token": TOKEN, "claudeExecutable": "/bin/false", "model": "haiku", "tiltDegrees": 0.4,
        "extraUrls": ["wss://bridge.example.com/v1"], "timeouts": {"firstOutputSec": 12}, "history": {"contextChars": 6000},
        "busyNotice": false,
        // written by a newer version: this one must keep it untouched
        "someFutureSetting": {"enabled": true, "level": 3},
    });
    std::fs::write(cfg_dir.join("config.json"), serde_json::to_string_pretty(&config).unwrap()).unwrap();
    std::fs::write(cfg_dir.join("history/0123456789abcdef.jsonl"), "{\"at\":\"2026-11-01T00:00:00Z\",\"q\":\"1+1は？\",\"a\":\"2です。\"}\n").unwrap();
    std::fs::write(cfg_dir.join("reports.jsonl"), "{\"at\":\"2026-11-01T00:00:00Z\",\"peer\":\"x\",\"data\":{}}\n").unwrap();

    // the first run: the same token opens the encrypted channel, and a device is added
    let b = Bridge::start(&exe, tmp, port);
    let r = spellink_bridge::probe::probe(&format!("ws://127.0.0.1:{port}/v1"), TOKEN).await.expect("the old token no longer works");
    assert_eq!(r.bridge, "upgrade");
    // no plugin package is configured: the device is added, its setup file cannot be written
    let added = Command::new(&exe).args(["device", "add", "Nomad"]).current_dir(tmp).env("HOME", tmp).env("XDG_CONFIG_HOME", tmp.join("cfg")).output().unwrap();
    assert!(!added.status.success() && String::from_utf8_lossy(&added.stderr).contains("the device was added but its setup file could not be written"), "{}", String::from_utf8_lossy(&added.stderr));
    b.stop();
    let before = snapshot(&cfg_dir);
    assert!(before.contains_key("devices.json"), "the ledger was written");

    // overwrite the same file, as an update does, and start it again
    std::fs::copy(BIN, &exe).unwrap();
    let b = Bridge::start(&exe, tmp, port);
    let r = spellink_bridge::probe::probe(&format!("ws://127.0.0.1:{port}/v1"), TOKEN).await.expect("the old token no longer works after the update");
    assert_eq!(r.bridge, "upgrade");
    let list = tokio::task::spawn_blocking({
        let (exe, tmp) = (exe.clone(), tmp.to_path_buf());
        move || run(&["device", "list"], &tmp, &exe)
    })
    .await
    .unwrap();
    assert!(list.contains("Nomad"), "the device survived the update: {list}");
    // …and then the user changes a setting on the device, which makes the bridge SAVE config.json.
    // That is where a key it does not know could be lost.
    let mut d = spellink_bridge_test_device(port).await;
    d.send(json!({"type": "setSettings", "settings": {"showProgress": true, "lineTilt": true, "thinkingAnimation": false, "fullWidthSpacing": 120, "halfWidthSpacing": 130, "busyNotice": false}})).await;
    loop {
        let m = d.recv().await.expect("the settings reply");
        if m["type"] == "settings" {
            break; // the save has happened
        }
    }
    b.stop();
    let after = snapshot(&cfg_dir);

    for (path, bytes) in &before {
        match after.get(path) {
            None => panic!("{path} was deleted by the update"),
            Some(got) if got != bytes && path != "config.json" && path != "devices.json" => panic!("{path} was changed by the update"),
            _ => {}
        }
    }
    let cfg: Value = serde_json::from_slice(&after["config.json"]).expect("config.json is no longer JSON");
    let mut want = config.clone();
    for (k, v) in [("showProgress", json!(true)), ("lineTilt", json!(true)), ("fullWidthSpacing", json!(120)), ("halfWidthSpacing", json!(130)), ("busyNotice", json!(false))] {
        want[k] = v;
    }
    for (k, v) in want.as_object().unwrap() {
        assert_eq!(&cfg[k], v, "config {k:?}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(cfg_dir.join("config.json")).unwrap().permissions().mode() & 0o777, 0o600, "config.json must stay readable by its owner only");
        assert_eq!(std::fs::metadata(cfg_dir.join("devices.json")).unwrap().permissions().mode() & 0o777, 0o600);
    }
}

/// A plugin session with the shared token, over the encrypted channel.
async fn spellink_bridge_test_device(port: u16) -> Dev {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    use spellink_bridge::securechan::{psk, Initiator};
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/v1")).await.unwrap();
    let (init, m1) = Initiator::start(&psk(TOKEN)).unwrap();
    ws.send(Message::Binary(m1.into())).await.unwrap();
    let Some(Ok(Message::Binary(m2))) = ws.next().await else { panic!("no handshake answer") };
    Dev { ws, ch: init.finish(&m2).unwrap() }
}

struct Dev {
    ws: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    ch: spellink_bridge::securechan::Channel,
}

impl Dev {
    async fn send(&mut self, v: Value) {
        use futures_util::SinkExt;
        let mut chunks = vec![];
        self.ch.seal(v.to_string().as_bytes(), |c| {
            chunks.push(c);
            Ok(())
        })
        .unwrap();
        for c in chunks {
            self.ws.send(tokio_tungstenite::tungstenite::Message::Binary(c.into())).await.unwrap();
        }
    }

    async fn recv(&mut self) -> Option<Value> {
        use futures_util::StreamExt;
        let mut buf = Vec::new();
        while let Some(Ok(m)) = self.ws.next().await {
            if let tokio_tungstenite::tungstenite::Message::Binary(ct) = m {
                if self.ch.open(&ct, &mut buf).ok()? {
                    return serde_json::from_slice(&buf).ok();
                }
            }
        }
        None
    }
}

//! `spellink-bridge device …`: the owner's commands for adding, pairing and removing devices (Go:
//! internal/devicecli). They talk to the running bridge over its local admin API (the pending
//! pairing requests live in that process), never to the ledger file.

use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Printed for a command line that is not understood.
pub const USAGE: &str = "usage: spellink-bridge device <command>
  add <name> [dir]        add a device and write its setup file (Spellink-<name>.snplg) into dir
  list                    show every device and its state
  pair --web              open the pairing page: read the QR code a device shows with this computer's camera
  pair [code]             confirm the 8-digit code a device is showing (asked for if not given)
  package <name> [dir]    write the setup file again (nothing changes)
  revoke <name>           stop a device from connecting, now and for good
  reissue <name>          give a device a new first secret; it must be paired again
  forget <name>           delete a revoked or expired device from the list";

/// Shows an address in the user's browser.
pub type Opener = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// The command line's connection to the running bridge.
pub struct Client {
    /// e.g. http://127.0.0.1:8765
    pub base: String,
    pub token: String,
    /// None: only the address is printed.
    pub open: Option<Opener>,
}

#[derive(Debug, Deserialize, Clone)]
struct Device {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    waiting: i64,
    #[serde(default)]
    expires: Option<String>,
    #[serde(default, rename = "lastSeen")]
    last_seen: Option<String>,
}

/// What the bridge answered when it refused.
#[derive(Debug)]
pub struct StatusError {
    pub status: u16,
    pub message: String,
}

impl std::fmt::Display for StatusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

enum Failure {
    Refused(StatusError),
    Other(String),
}

impl From<Failure> for String {
    fn from(f: Failure) -> String {
        match f {
            Failure::Refused(e) => e.message,
            Failure::Other(m) => m,
        }
    }
}

fn split_base(base: &str) -> Result<(String, String), String> {
    let rest = base.strip_prefix("http://").ok_or_else(|| format!("{base}: only http:// addresses (the bridge on this computer)"))?;
    let host = rest.trim_end_matches('/');
    if host.is_empty() || host.contains('/') || host.contains('[') && !host.contains(']') {
        return Err(format!("{base}: not a usable address"));
    }
    Ok((host.to_string(), format!("Host: {host}\r\n")))
}

const MAX_ANSWER: usize = 64 << 20;

impl Client {
    async fn do_request(&self, method: &str, path: &str, body: Option<Value>) -> Result<(u16, Vec<u8>), Failure> {
        let (host, host_header) = split_base(&self.base).map_err(Failure::Other)?;
        let unreachable = |e: String| Failure::Other(format!("cannot reach the bridge at {} (is it running? start it with `spellink-bridge serve` or the service): {e}", self.base));
        let work = async {
            let mut s = tokio::net::TcpStream::connect(&host).await.map_err(|e| unreachable(e.to_string()))?;
            let payload = body.map(|b| b.to_string());
            let mut req = format!("{method} {path} HTTP/1.1\r\n{host_header}Authorization: Bearer {}\r\nConnection: close\r\n", self.token);
            if let Some(p) = &payload {
                req += &format!("Content-Type: application/json\r\nContent-Length: {}\r\n", p.len());
            }
            req += "\r\n";
            s.write_all(req.as_bytes()).await.map_err(|e| unreachable(e.to_string()))?;
            if let Some(p) = payload {
                s.write_all(p.as_bytes()).await.map_err(|e| unreachable(e.to_string()))?;
            }
            let mut raw = Vec::new();
            (&mut s).take(MAX_ANSWER as u64 + 1).read_to_end(&mut raw).await.map_err(|e| Failure::Other(e.to_string()))?;
            Ok::<_, Failure>(raw)
        };
        let raw = tokio::time::timeout(Duration::from_secs(30), work).await.map_err(|_| Failure::Other("the bridge did not answer in time".into()))??;
        let (status, out) = parse_response(&raw).map_err(Failure::Other)?;
        if status >= 300 {
            let mut msg = String::from_utf8_lossy(&out).trim().to_string();
            if status == 401 {
                msg = "the bridge did not accept the admin token (admin.token in its config directory)".into();
            }
            return Err(Failure::Refused(StatusError { status, message: msg }));
        }
        Ok((status, out))
    }

    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<(u16, Vec<u8>), String> {
        self.do_request(method, path, body).await.map_err(String::from)
    }
}

/// Reads an HTTP/1.1 answer. One that is cut off (shorter than it said) is an error, not a result.
fn parse_response(raw: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let at = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or("the bridge's answer was cut off")?;
    let head = String::from_utf8_lossy(&raw[..at]).into_owned();
    let body = &raw[at + 4..];
    let status: u16 = head.split(' ').nth(1).and_then(|s| s.parse().ok()).ok_or("the bridge's answer has no status")?;
    let header = |name: &str| head.lines().skip(1).find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_string()));
    if header("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        let mut out = Vec::new();
        let mut rest = body;
        loop {
            let line_end = rest.windows(2).position(|w| w == b"\r\n").ok_or("the bridge's answer was cut off")?;
            let size = usize::from_str_radix(String::from_utf8_lossy(&rest[..line_end]).trim(), 16).map_err(|_| "the bridge's answer is not valid HTTP")?;
            rest = &rest[line_end + 2..];
            if size == 0 {
                return Ok((status, out));
            }
            if rest.len() < size + 2 {
                return Err("the bridge's answer was cut off".into());
            }
            out.extend_from_slice(&rest[..size]);
            rest = &rest[size + 2..];
        }
    }
    if let Some(n) = header("content-length").and_then(|v| v.parse::<usize>().ok()) {
        if body.len() < n {
            return Err("the bridge's answer was cut off".into());
        }
        return Ok((status, body[..n].to_vec()));
    }
    Ok((status, body.to_vec()))
}

/// Executes one `device` command. `dir` is where setup files are written when none is named.
/// `read_line` supplies a line of input when the code must be asked for.
pub async fn run(c: &Client, args: &[String], read_line: &mut dyn FnMut() -> Option<String>, out: &mut dyn Write, dir: &Path) -> Result<(), String> {
    let Some(cmd) = args.first() else { return Err(USAGE.into()) };
    let rest = &args[1..];
    let pick = |i: usize| rest.get(i).map(PathBuf::from).unwrap_or_else(|| dir.to_path_buf());
    match cmd.as_str() {
        "add" => {
            if rest.is_empty() || rest.len() > 2 {
                return Err("usage: spellink-bridge device add <name> [dir]".into());
            }
            add(c, &rest[0], &pick(1), out).await
        }
        "list" => list(c, out).await,
        "pair" => {
            if rest.len() == 1 && rest[0] == "--web" {
                return open_page(
                    c,
                    "/pair",
                    "Opened the pairing page in your browser. Show the QR code on the device to the camera.",
                    "Open this address in a browser on this computer, then show the QR code on the device to the camera:",
                    out,
                )
                .await;
            }
            pair(c, rest.first().map(String::as_str).unwrap_or(""), read_line, out).await
        }
        "package" => {
            if rest.is_empty() || rest.len() > 2 {
                return Err("usage: spellink-bridge device package <name> [dir]".into());
            }
            let d = find(c, &rest[0]).await?;
            let path = write_package(c, &d, &pick(1)).await?;
            writeln!(out, "Wrote {}", path.display()).map_err(|e| e.to_string())
        }
        "revoke" | "reissue" | "forget" => {
            if rest.len() != 1 {
                return Err(format!("usage: spellink-bridge device {cmd} <name>"));
            }
            act(c, cmd, &rest[0], out).await
        }
        _ => Err(USAGE.into()),
    }
}

async fn fetch(c: &Client) -> Result<Vec<Device>, String> {
    let (_, body) = c.call("GET", "/admin/devices", None).await?;
    serde_json::from_slice(&body).map_err(|e| format!("the bridge sent a list this version cannot read: {e}"))
}

/// Resolves a device by its exact name or its id.
async fn find(c: &Client, name_or_id: &str) -> Result<Device, String> {
    fetch(c).await?.into_iter().find(|d| d.name == name_or_id || d.id == name_or_id).ok_or_else(|| format!("no device named {name_or_id:?} (see `spellink-bridge device list`)"))
}

/// The setup file's name: the device's name made safe for any file system.
fn file_name(name: &str, id: &str) -> String {
    let mut safe = String::new();
    let mut in_run = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            safe.push(ch);
            in_run = false;
        } else if !in_run {
            safe.push('_');
            in_run = true;
        }
    }
    let n = safe.trim_matches(|c| matches!(c, '.' | '_' | '-'));
    format!("Spellink-{}.snplg", if n.is_empty() { id } else { n })
}

/// Downloads the device's setup file into `dir`. It never overwrites a file: the file carries a
/// secret until the device is paired, and a name clash must not destroy another.
async fn write_package(c: &Client, d: &Device, dir: &Path) -> Result<PathBuf, String> {
    let (_, data) = c.call("GET", &format!("/admin/devices/{}/package", d.id), None).await?;
    let path = dir.join(file_name(&d.name, &d.id));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let mut f = opts.open(&path).map_err(|e| format!("cannot write {}: {e} (remove it first if you want a new one)", path.display()))?;
    let written = f.write_all(&data).and_then(|_| f.sync_all());
    drop(f);
    if let Err(e) = written {
        let _ = std::fs::remove_file(&path);
        return Err(e.to_string());
    }
    // Success is checked by the result: the file must be the size that was sent.
    if std::fs::metadata(&path).map(|m| m.len()).ok() != Some(data.len() as u64) {
        return Err(format!("{} was not written completely", path.display()));
    }
    Ok(path)
}

async fn add(c: &Client, name: &str, dir: &Path, out: &mut dyn Write) -> Result<(), String> {
    let (_, body) = c.call("POST", "/admin/devices", Some(json!({"name": name}))).await?;
    let created: Value = serde_json::from_slice(&body).unwrap_or_default();
    let (Some(id), cname) = (created["id"].as_str().filter(|s| !s.is_empty()), created["name"].as_str().unwrap_or(name)) else {
        return Err("the bridge's answer to \"add\" could not be read".into());
    };
    let d = Device { id: id.into(), name: cname.into(), status: String::new(), waiting: 0, expires: None, last_seen: None };
    let path = write_package(c, &d, dir).await.map_err(|e| format!("the device was added but its setup file could not be written: {e}\n(write it again with `spellink-bridge device package {name:?}`)"))?;
    let w = |e: std::io::Error| e.to_string();
    writeln!(out, "Added {name:?}. Setup file: {}", path.display()).map_err(w)?;
    writeln!(out, "  1. Copy the file to the device's MyStyle folder, then install it on the device (Settings > Apps > Plugins).").map_err(w)?;
    writeln!(out, "  2. On the device, open Spellink's settings and choose pairing: an 8-digit code and a QR code appear.").map_err(w)?;
    writeln!(out, "  3. While the code is shown (90 seconds by default), run `spellink-bridge device pair` here and type it.").map_err(w)?;
    writeln!(out, "  4. `spellink-bridge device list` shows the device as active.").map_err(w)?;
    writeln!(out, "The file works only for this device, only until it is paired (24 hours by default). Keep it private until then.").map_err(w)
}

/// A stored time as "2006-01-02 15:04" in UTC (the local zone is the bridge's job to know).
fn minute(t: &Option<String>) -> String {
    match t.as_deref().and_then(|s| crate::timestamp::Timestamp::parse(s).ok()) {
        Some(ts) if !ts.is_zero() => {
            let s = crate::util::rfc3339_utc(ts.unix());
            format!("{} {}", &s[..10], &s[11..16])
        }
        _ => String::new(),
    }
}

async fn list(c: &Client, out: &mut dyn Write) -> Result<(), String> {
    let devs = fetch(c).await?;
    if devs.is_empty() {
        return writeln!(out, "No devices yet. Add one with `spellink-bridge device add <name>`.").map_err(|e| e.to_string());
    }
    let mut rows = vec![["NAME".to_string(), "ID".into(), "STATE".into(), "NOTE".into()]];
    for d in &devs {
        let mut note = String::new();
        match d.status.as_str() {
            "issued" | "waiting" | "granting" => {
                note = format!("setup file valid until {}", minute(&d.expires));
                if d.waiting > 0 {
                    note = format!("{} waiting for a code; {note}", d.waiting);
                }
            }
            "active" => {
                let m = minute(&d.last_seen);
                if !m.is_empty() {
                    note = format!("last seen {m}");
                }
            }
            _ => {}
        }
        rows.push([d.name.clone(), d.id.clone(), d.status.clone(), note]);
    }
    let widths: Vec<usize> = (0..3).map(|i| rows.iter().map(|r| r[i].chars().count()).max().unwrap_or(0) + 2).collect();
    for r in rows {
        let line = format!("{:<w0$}{:<w1$}{:<w2$}{}", r[0], r[1], r[2], r[3], w0 = widths[0], w1 = widths[1], w2 = widths[2]);
        writeln!(out, "{}", line.trim_end()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

async fn pair(c: &Client, code: &str, read_line: &mut dyn FnMut() -> Option<String>, out: &mut dyn Write) -> Result<(), String> {
    let code = if code.is_empty() {
        write!(out, "Code shown on the device: ").map_err(|e| e.to_string())?;
        out.flush().map_err(|e| e.to_string())?;
        read_line().map(|l| l.trim().to_string()).ok_or("no code was given")?
    } else {
        code.to_string()
    };
    let body = match c.do_request("POST", "/admin/pair", Some(json!({"code": code}))).await {
        Ok((_, b)) => b,
        Err(Failure::Refused(e)) if e.status == 403 => return Err("that code was not accepted; check the device's screen and try again".into()),
        Err(e) => return Err(e.into()),
    };
    let res: Value = serde_json::from_slice(&body).unwrap_or_default();
    let name = res["name"].as_str().filter(|n| !n.is_empty()).ok_or("the bridge's answer to \"pair\" could not be read")?;
    writeln!(out, "Code accepted: {name:?} is being paired. The device finishes by itself.").map_err(|e| e.to_string())
}

/// Opens the bridge's app page (status and devices) in the browser: `spellink-bridge open`.
pub async fn open_app(c: &Client, out: &mut dyn Write) -> Result<(), String> {
    open_page(c, "/app", "Opened the Spellink Bridge App in your browser.", "Open this address in a browser on this computer:", out).await
}

/// Opens one of the bridge's local pages. The command line's token travels in the part of the
/// address after # — a browser never sends that to a server — so the page can use it and nothing
/// else sees it.
async fn open_page(c: &Client, path: &str, opened: &str, not_opened: &str, out: &mut dyn Write) -> Result<(), String> {
    // ask the bridge something first: a bridge that is not running should be said, not left to a blank browser tab
    fetch(c).await?;
    let url = format!("{}{path}#{}", c.base, c.token);
    if let Some(open) = &c.open {
        if open(&url).is_ok() {
            return writeln!(out, "{opened}").map_err(|e| e.to_string());
        }
    }
    writeln!(out, "{not_opened}").map_err(|e| e.to_string())?;
    writeln!(out, "  {url}").map_err(|e| e.to_string())
}

async fn act(c: &Client, cmd: &str, name_or_id: &str, out: &mut dyn Write) -> Result<(), String> {
    let d = find(c, name_or_id).await?;
    c.call("POST", &format!("/admin/devices/{}/{cmd}", d.id), None).await?;
    match cmd {
        "revoke" => writeln!(out, "Revoked {:?}. It cannot connect any more.", d.name),
        "reissue" => writeln!(out, "Reissued {:?}: pair it again with `spellink-bridge device package {:?}`.", d.name, d.name),
        _ => writeln!(out, "Forgot {:?}.", d.name),
    }
    .map_err(|e| e.to_string())
}

/// How this platform opens an address in the default browser.
pub fn open_command(os: &str, url: &str) -> (&'static str, Vec<String>) {
    match os {
        "macos" => ("open", vec![url.into()]),
        "windows" => ("rundll32", vec!["url.dll,FileProtocolHandler".into(), url.into()]),
        _ => ("xdg-open", vec![url.into()]),
    }
}

/// Starts the default browser and does not wait for it.
pub fn open_browser(url: &str) -> Result<(), String> {
    let (name, args) = open_command(std::env::consts::OS, url);
    let mut child = std::process::Command::new(name).args(args).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().map_err(|e| format!("{name}: {e}"))?;
    std::thread::spawn(move || {
        let _ = child.wait(); // reap it; the browser keeps running
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_safe_for_any_file_system() {
        for (name, want) in
            [("Nomad", "Spellink-Nomad.snplg"), ("My Nomad!", "Spellink-My_Nomad.snplg"), ("../../etc/passwd", "Spellink-etc_passwd.snplg"), ("ノート", "Spellink-0123.snplg"), ("...", "Spellink-0123.snplg")]
        {
            assert_eq!(file_name(name, "0123"), want, "file_name({name:?})");
        }
    }

    #[test]
    fn an_answer_that_is_cut_off_is_an_error() {
        assert!(parse_response(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nshort").is_err(), "a truncated answer must not be taken as complete");
        assert!(parse_response(b"HTTP/1.1 200 OK\r\nContent-Le").is_err());
        assert_eq!(parse_response(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello").unwrap(), (200, b"hello".to_vec()));
        assert_eq!(parse_response(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n").unwrap(), (200, b"hello".to_vec()));
        assert!(parse_response(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhel").is_err());
    }

    #[test]
    fn the_browser_command_depends_on_the_platform() {
        assert_eq!(open_command("macos", "http://x").0, "open");
        assert_eq!(open_command("windows", "http://x").0, "rundll32");
        assert_eq!(open_command("linux", "http://x"), ("xdg-open", vec!["http://x".to_string()]));
    }

    #[test]
    fn only_a_plain_http_address_is_usable() {
        assert!(split_base("http://127.0.0.1:8765").is_ok());
        for bad in ["https://x", "http://[::1", "ftp://x", "127.0.0.1:8765", "http://"] {
            assert!(split_base(bad).is_err(), "{bad}");
        }
    }
}

//! The smaller commands of the command line (Go: main.go's pair and check.go).

use crate::app::pair_urls;
use crate::config::Store;
use crate::probe;
use std::time::Duration;

/// Prints or writes the connection file the plugin reads (MyStyle/Spellink/bridges.json).
pub fn pair(out: Option<&str>) -> Result<(), String> {
    let cfg = Store::user()?.load()?;
    let body = serde_json::to_string_pretty(&serde_json::json!({
        "bridges": [{"name": cfg.name, "token": cfg.token, "urls": pair_urls(&cfg)}]
    }))
    .map_err(|e| e.to_string())?;
    match out {
        None | Some("") => println!("{body}"),
        Some(path) => {
            crate::util::write_private(std::path::Path::new(path), format!("{body}\n").as_bytes()).map_err(|e| format!("{path}: {e}"))?;
            println!("Wrote {path}");
        }
    }
    Ok(())
}

/// Dials the given addresses (by default every address `pair` would hand to the plugin) with this
/// bridge's token and reports, for each, whether the plugin would get through.
pub async fn check(urls: &[String]) -> Result<(), String> {
    let cfg = Store::user()?.load()?;
    let urls: Vec<String> = if urls.is_empty() { pair_urls(&cfg) } else { urls.to_vec() };
    let mut ok = 0;
    for u in &urls {
        match tokio::time::timeout(Duration::from_secs(8), probe::probe(u, &cfg.token)).await {
            Ok(Ok(r)) => {
                ok += 1;
                println!("OK  {u}\n      bridge {:?}, version {}, protocol {}, {} ms", r.bridge, r.version, r.protocol, r.elapsed.as_millis());
            }
            Ok(Err(e)) => println!("NG  {u}\n      {e}"),
            Err(_) => println!("NG  {u}\n      timed out"),
        }
    }
    if ok == 0 {
        return Err("no address could be reached".into());
    }
    Ok(())
}

/// The command line's connection to the running bridge on this computer.
fn admin_client() -> Result<crate::devicecli::Client, String> {
    let store = Store::user()?;
    let cfg = store.load()?;
    let token = store.admin_token()?;
    Ok(crate::devicecli::Client { base: format!("http://127.0.0.1:{}", cfg.port), token, open: Some(std::sync::Arc::new(|u| crate::devicecli::open_browser(u))) })
}

/// `spellink-bridge device …`.
pub async fn device(args: &[String]) -> Result<(), String> {
    let c = admin_client()?;
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let mut stdout = std::io::stdout();
    crate::devicecli::run(
        &c,
        args,
        &mut || {
            let mut line = String::new();
            // at most 256 bytes of one line, like the Go command
            match std::io::stdin().read_line(&mut line) {
                Ok(0) | Err(_) => None,
                Ok(_) => Some(line.chars().take(256).collect()),
            }
        },
        &mut stdout,
        &cwd,
    )
    .await
}

/// `spellink-bridge open`: the app page in the browser.
pub async fn open_app() -> Result<(), String> {
    let c = admin_client()?;
    crate::devicecli::open_app(&c, &mut std::io::stdout()).await
}

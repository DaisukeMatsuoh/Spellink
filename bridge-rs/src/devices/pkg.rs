//! Adds a device's pairing setup to the plugin package (Go: internal/devices/pkg.go).

use super::registry::PackageData;
use super::{valid_id, valid_name, valid_secret, MAX_GEN};
use serde::Serialize;
use std::io::{Cursor, Read, Write};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

/// The JavaScript bundle inside a plugin package.
pub const BUNDLE_NAME: &str = "Spellink.bundle";

pub const PAIR_MARKER: &str = "globalThis.__SPELLINK_PAIR__=";

// What the plugin accepts in the pairing setup (the plugin checks the same limits again).
pub const MAX_URLS: usize = 8;
pub const MAX_URL_CHARS: usize = 512;
pub const MIN_WINDOW_SECS: u64 = 10;
pub const MAX_WINDOW_SECS: u64 = 600;

/// Bounds one file inside a package.
pub const MAX_PACKAGE: u64 = 64 << 20;

/// The line written at the top of the bundle: what the plugin needs to pair (ADR 0016).
#[derive(Serialize)]
struct PairSetup<'a> {
    v: u32,
    id: &'a str,
    #[serde(rename = "gen")]
    generation: i64,
    #[serde(skip_serializing_if = "str::is_empty")]
    b: &'a str,
    name: &'a str,
    urls: &'a [String],
    #[serde(rename = "windowSec")]
    window_sec: u64,
}

/// JSON that is safe inside a `<script>`: like Go's encoder, `<`, `>`, `&` and the two line
/// separators are written as escapes (they can only occur inside strings).
fn script_safe(json: String) -> String {
    json.replace('<', "\\u003c").replace('>', "\\u003e").replace('&', "\\u0026").replace('\u{2028}', "\\u2028").replace('\u{2029}', "\\u2029")
}

fn read_entry(z: &mut ZipArchive<Cursor<&[u8]>>, i: usize, limit: u64) -> Result<Vec<u8>, String> {
    let mut f = z.by_index(i).map_err(|e| e.to_string())?;
    if f.size() > limit {
        return Err("package: an entry is too large".into());
    }
    // the reader checks the entry's checksum when it reaches the end, and what is read is bounded
    // by the size in the header (bounded above)
    let mut out = Vec::with_capacity(f.size() as usize);
    (&mut f).take(limit + 1).read_to_end(&mut out).map_err(|e| e.to_string())?;
    if out.len() as u64 > limit {
        return Err("package: an entry is too large".into());
    }
    Ok(out)
}

/// A copy of the plugin package `base` (.snplg, a zip) whose bundle starts with the device's
/// pairing setup. Every other file is copied as it is. The result is read back and checked before
/// it is returned.
pub fn build_package(base: &[u8], d: &PackageData, urls: &[String]) -> Result<Vec<u8>, String> {
    build_package_limited(base, d, urls, MAX_PACKAGE, &verify_package)
}

pub(super) fn build_package_limited(
    base: &[u8],
    d: &PackageData,
    urls: &[String],
    limit: u64,
    verify: &dyn Fn(&[u8], &[u8], &str, u64) -> Result<(), String>,
) -> Result<Vec<u8>, String> {
    let window_sec = d.window.as_secs();
    if !valid_id(&d.id) || !valid_name(&d.name) || d.generation < 1 || d.generation > MAX_GEN {
        return Err("package: bad device data".into());
    }
    if !d.b.is_empty() && !valid_secret(&d.b) {
        return Err("package: bad B".into());
    }
    if urls.len() > MAX_URLS || (urls.is_empty() && !d.b.is_empty()) {
        // a package that still pairs needs somewhere to connect; one for a paired device does not
        // (it connects with what it stored when it was paired)
        return Err(format!("package: {} URLs (a device that still has to pair needs 1 to {MAX_URLS})", urls.len()));
    }
    if !(MIN_WINDOW_SECS..=MAX_WINDOW_SECS).contains(&window_sec) {
        return Err(format!("package: window of {window_sec} s (need {MIN_WINDOW_SECS} to {MAX_WINDOW_SECS})"));
    }
    for u in urls {
        if !u.starts_with("wss://") || u.len() > MAX_URL_CHARS || u.contains(['"', '\\', '\n', '\r']) {
            return Err(format!("package: URL {u:?} is not usable (wss:// only, up to {MAX_URL_CHARS} characters)"));
        }
    }
    let setup = PairSetup { v: 1, id: &d.id, generation: d.generation, b: &d.b, name: &d.name, urls, window_sec };
    let js = script_safe(serde_json::to_string(&setup).map_err(|e| e.to_string())?);
    let line = format!("{PAIR_MARKER}{js};\n");

    let mut zr = ZipArchive::new(Cursor::new(base)).map_err(|e| format!("package: the base is not a zip: {e}"))?;
    let mut zw = ZipWriter::new(Cursor::new(Vec::new()));
    let mut found = false;
    for i in 0..zr.len() {
        let (name, modified) = {
            let f = zr.by_index_raw(i).map_err(|e| e.to_string())?;
            (f.name().map_err(|e| e.to_string())?.into_owned(), f.last_modified())
        };
        if name != BUNDLE_NAME {
            let f = zr.by_index_raw(i).map_err(|e| e.to_string())?;
            zw.raw_copy_file(f).map_err(|e| e.to_string())?;
            continue;
        }
        found = true;
        let body = read_entry(&mut zr, i, limit)?;
        if find(&body, PAIR_MARKER.as_bytes()) {
            return Err("package: the base already carries a pairing setup; use the original plugin package".into());
        }
        let mut opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        if let Some(m) = modified {
            opts = opts.last_modified_time(m);
        }
        zw.start_file(BUNDLE_NAME, opts).map_err(|e| e.to_string())?;
        zw.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
        zw.write_all(&body).map_err(|e| e.to_string())?;
    }
    if !found {
        return Err(format!("package: {BUNDLE_NAME} not found in the base"));
    }
    let out = zw.finish().map_err(|e| e.to_string())?.into_inner();
    verify(&out, base, &line, limit).map_err(|e| format!("package: written file does not check out: {e}"))?;
    Ok(out)
}

fn find(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// Reads the result back: same entries, the bundle starts with the line, the rest is unchanged.
pub fn verify_package(data: &[u8], base: &[u8], line: &str, limit: u64) -> Result<(), String> {
    let mut zr = ZipArchive::new(Cursor::new(data)).map_err(|e| e.to_string())?;
    let mut br = ZipArchive::new(Cursor::new(base)).map_err(|e| e.to_string())?;
    if zr.len() != br.len() {
        return Err(format!("{} entries, expected {}", zr.len(), br.len()));
    }
    for i in 0..zr.len() {
        let name = zr.by_index_raw(i).map_err(|e| e.to_string())?.name().map_err(|e| e.to_string())?.into_owned();
        let want_name = br.by_index_raw(i).map_err(|e| e.to_string())?.name().map_err(|e| e.to_string())?.into_owned();
        if name != want_name {
            return Err(format!("entry {i} is {name:?}, expected {want_name:?}"));
        }
        let got = read_entry(&mut zr, i, limit)?;
        let mut want = read_entry(&mut br, i, limit)?;
        if name == BUNDLE_NAME {
            want = [line.as_bytes(), &want].concat();
        }
        if got != want {
            return Err(format!("entry {name:?} differs"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn secret(n: u8) -> String {
        use base64::Engine;
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([n; 32])
    }

    fn make_zip(files: &[(&str, &str)], method: CompressionMethod) -> Vec<u8> {
        let mut zw = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, body) in files {
            let opts = SimpleFileOptions::default().compression_method(method).last_modified_time(zip::DateTime::from_date_and_time(2026, 11, 8, 19, 9, 0).unwrap());
            zw.start_file(*name, opts).unwrap();
            zw.write_all(body.as_bytes()).unwrap();
        }
        zw.finish().unwrap().into_inner()
    }

    fn deflated(files: &[(&str, &str)]) -> Vec<u8> {
        make_zip(files, CompressionMethod::Deflated)
    }

    fn entries(data: &[u8]) -> Vec<(String, String)> {
        let mut z = ZipArchive::new(Cursor::new(data)).unwrap();
        (0..z.len())
            .map(|i| {
                let name = z.by_index_raw(i).unwrap().name().unwrap().into_owned();
                (name, String::from_utf8(read_entry(&mut z, i, MAX_PACKAGE).unwrap()).unwrap())
            })
            .collect()
    }

    fn entry(data: &[u8], name: &str) -> String {
        entries(data).into_iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no entry {name}")).1
    }

    fn good_data() -> PackageData {
        PackageData { id: "0123456789abcdef".into(), name: "Nomad".into(), generation: 3, b: secret(5), window: Duration::from_secs(90) }
    }

    fn good_urls() -> Vec<String> {
        vec!["wss://bridge.example/v1?d=0123456789abcdef".into(), "wss://relay.example/v1/relay/client?room=abc".into()]
    }

    fn base_zip() -> Vec<u8> {
        deflated(&[("PluginConfig.json", r#"{"name":"Spellink"}"#), (BUNDLE_NAME, "var __BUNDLE_START_TIME__=1;"), ("icon.png", "PNG")])
    }

    #[test]
    fn the_setup_goes_at_the_top_of_the_bundle_and_nothing_else_changes() {
        let out = build_package(&base_zip(), &good_data(), &good_urls()).unwrap();
        let got = entries(&out);
        assert_eq!(got.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["PluginConfig.json", BUNDLE_NAME, "icon.png"], "entry order changed");
        assert_eq!(got[0].1, r#"{"name":"Spellink"}"#);
        assert_eq!(got[2].1, "PNG");
        let (line, rest) = got[1].1.split_once('\n').unwrap();
        assert_eq!(rest, "var __BUNDLE_START_TIME__=1;");
        assert!(line.starts_with(PAIR_MARKER) && line.ends_with(';'), "{line}");
        let v: serde_json::Value = serde_json::from_str(line.strip_prefix(PAIR_MARKER).unwrap().strip_suffix(';').unwrap()).unwrap();
        let d = good_data();
        assert_eq!((v["v"].clone(), v["id"].clone(), v["gen"].clone(), v["b"].clone(), v["name"].clone(), v["windowSec"].clone()), (1.into(), d.id.into(), 3.into(), d.b.into(), "Nomad".into(), 90.into()));
        assert_eq!(v["urls"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn a_package_for_a_paired_device_carries_no_secret_and_needs_no_url() {
        let d = PackageData { b: String::new(), ..good_data() };
        let out = build_package(&base_zip(), &d, &good_urls()).unwrap();
        assert!(!entry(&out, BUNDLE_NAME).contains("\"b\""), "a paired device's package must not carry B");
        let out = build_package(&base_zip(), &d, &[]).unwrap();
        let bundle = entry(&out, BUNDLE_NAME);
        assert!(bundle.split_once('\n').unwrap().0.contains("\"urls\":[]"), "urls must be an empty list, not null");
    }

    #[test]
    fn what_it_writes_is_escaped_for_a_script() {
        let d = PackageData { name: "</script>\u{2028}\u{2029} &".into(), ..good_data() };
        let out = build_package(&base_zip(), &d, &good_urls()).unwrap();
        let bundle = entry(&out, BUNDLE_NAME);
        let line = bundle.split_once('\n').unwrap().0;
        for bad in ["</script>", "\u{2028}", "\u{2029}", "<", ">", "&"] {
            assert!(!line.contains(bad), "the setup line contains {bad:?} raw");
        }
        let v: serde_json::Value = serde_json::from_str(line.strip_prefix(PAIR_MARKER).unwrap().strip_suffix(';').unwrap()).unwrap();
        assert_eq!(v["name"], "</script>\u{2028}\u{2029} &", "the escapes mean the same text");
    }

    #[test]
    fn bad_input_is_refused() {
        type Mutation = Box<dyn Fn(&mut PackageData, &mut Vec<String>)>;
        let cases: Vec<(&str, Mutation)> = vec![
            ("bad id", Box::new(|d, _| d.id = "x".into())),
            ("bad name", Box::new(|d, _| d.name = String::new())),
            ("gen 0", Box::new(|d, _| d.generation = 0)),
            ("bad B", Box::new(|d, _| d.b = "short".into())),
            ("no URLs", Box::new(|_, u| u.clear())),
            ("too many URLs", Box::new(|_, u| *u = vec!["wss://x".into(); MAX_URLS + 1])),
            ("plain ws", Box::new(|_, u| *u = vec!["ws://x/v1".into()])),
            ("https", Box::new(|_, u| *u = vec!["https://x/v1".into()])),
            ("quote in URL", Box::new(|_, u| *u = vec!["wss://x/\"".into()])),
            ("backslash in URL", Box::new(|_, u| *u = vec!["wss://x/\\".into()])),
            ("newline in URL", Box::new(|_, u| *u = vec!["wss://x/\n".into()])),
            ("URL too long", Box::new(|_, u| *u = vec![format!("wss://{}", "a".repeat(MAX_URL_CHARS))])),
            ("window too short", Box::new(|d, _| d.window = Duration::from_secs(5))),
            ("window too long", Box::new(|d, _| d.window = Duration::from_secs(660))),
        ];
        for (name, f) in cases {
            let (mut d, mut u) = (good_data(), good_urls());
            f(&mut d, &mut u);
            assert!(build_package(&base_zip(), &d, &u).is_err(), "{name}: no error");
        }
    }

    #[test]
    fn a_base_it_cannot_use_is_refused() {
        assert!(build_package(b"not a zip", &good_data(), &good_urls()).is_err());
        let no_bundle = deflated(&[("icon.png", "x")]);
        assert!(build_package(&no_bundle, &good_data(), &good_urls()).unwrap_err().contains("not found"));
        let once = build_package(&base_zip(), &good_data(), &good_urls()).unwrap();
        assert!(build_package(&once, &good_data(), &good_urls()).unwrap_err().contains("already"), "a base that already has a setup");
    }

    #[test]
    fn an_oversized_entry_is_refused() {
        assert!(build_package_limited(&base_zip(), &good_data(), &good_urls(), 10, &verify_package).is_err());
    }

    /// Patches the compression method of every entry to 99 (no such method).
    fn with_unknown_method(mut data: Vec<u8>) -> Vec<u8> {
        for i in 0..data.len().saturating_sub(10) {
            if data[i..i + 4] == [0x50, 0x4b, 0x03, 0x04] {
                data[i + 8] = 99;
            }
            if data[i..i + 4] == [0x50, 0x4b, 0x01, 0x02] {
                data[i + 10] = 99;
            }
        }
        data
    }

    #[test]
    fn an_entry_it_cannot_open_is_refused() {
        let base = with_unknown_method(make_zip(&[(BUNDLE_NAME, "xyz")], CompressionMethod::Stored));
        assert!(build_package(&base, &good_data(), &good_urls()).is_err(), "an entry with an unknown compression method must be refused");
    }

    #[test]
    fn bytes_that_fail_their_check_are_not_returned() {
        let r = build_package_limited(&base_zip(), &good_data(), &good_urls(), MAX_PACKAGE, &|_, _, _, _| Err("injected".into()));
        assert!(r.unwrap_err().contains("injected"));
    }

    #[test]
    fn verify_catches_every_kind_of_damage() {
        let base = base_zip();
        let line = "L;\n";
        let good = deflated(&[("PluginConfig.json", r#"{"name":"Spellink"}"#), (BUNDLE_NAME, &format!("{line}var __BUNDLE_START_TIME__=1;")), ("icon.png", "PNG")]);
        assert_eq!(verify_package(&good, &base, line, MAX_PACKAGE), Ok(()), "a correct package");
        assert!(verify_package(b"junk", &base, line, MAX_PACKAGE).is_err(), "not a zip");
        assert!(verify_package(&deflated(&[("a", "x")]), &base, line, MAX_PACKAGE).is_err(), "wrong number of entries");
        let renamed = deflated(&[("x", "1"), (BUNDLE_NAME, &format!("{line}var __BUNDLE_START_TIME__=1;")), ("icon.png", "PNG")]);
        assert!(verify_package(&renamed, &base, line, MAX_PACKAGE).is_err(), "wrong entry name");
        let changed = deflated(&[("PluginConfig.json", "CHANGED"), (BUNDLE_NAME, &format!("{line}var __BUNDLE_START_TIME__=1;")), ("icon.png", "PNG")]);
        assert!(verify_package(&changed, &base, line, MAX_PACKAGE).is_err(), "a changed file");
        let no_line = deflated(&[("PluginConfig.json", r#"{"name":"Spellink"}"#), (BUNDLE_NAME, "no setup line"), ("icon.png", "PNG")]);
        assert!(verify_package(&no_line, &base, line, MAX_PACKAGE).is_err(), "a bundle without the line");
    }

    fn stored(files: &[(&str, &str)]) -> Vec<u8> {
        make_zip(files, CompressionMethod::Stored)
    }

    fn replace_once(data: &[u8], from: &str, to: &str) -> Vec<u8> {
        let at = data.windows(from.len()).position(|w| w == from.as_bytes()).expect("the text is in the zip");
        let mut out = data.to_vec();
        out[at..at + to.len()].copy_from_slice(to.as_bytes());
        out
    }

    #[test]
    fn a_damaged_base_is_refused() {
        let data = stored(&[("PluginConfig.json", "{}"), (BUNDLE_NAME, "BUNDLEBUNDLEBUNDLE")]);
        let damaged = replace_once(&data, "BUNDLEBUNDLEBUNDLE", "BUNDLEBUNDLEBUNDLX"); // the checksum no longer matches
        assert!(build_package(&damaged, &good_data(), &good_urls()).is_err(), "a bundle that fails its checksum must be refused");
    }

    #[test]
    fn verify_notices_an_unreadable_entry() {
        let base = base_zip();
        let bad = replace_once(&stored(&[("PluginConfig.json", "ABCD"), (BUNDLE_NAME, "x"), ("icon.png", "PNG")]), "ABCD", "ABCE");
        assert!(verify_package(&bad, &base, "L", MAX_PACKAGE).is_err(), "an entry that fails its checksum in the result");
        // and one in the base itself
        let good = stored(&[("PluginConfig.json", r#"{"name":"Spellink"}"#), (BUNDLE_NAME, "L;\nvar __BUNDLE_START_TIME__=1;"), ("icon.png", "PNG")]);
        let bad_base = replace_once(&stored(&[("PluginConfig.json", "WXYZ"), (BUNDLE_NAME, "var __BUNDLE_START_TIME__=1;"), ("icon.png", "PNG")]), "WXYZ", "WXYQ");
        assert!(verify_package(&good, &bad_base, "L;\n", MAX_PACKAGE).is_err(), "a base entry that fails its checksum");
    }
}

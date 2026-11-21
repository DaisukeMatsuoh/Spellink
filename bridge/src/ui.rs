//! The two pages the bridge serves to the person at this computer (Go: server/pairpage and
//! server/appui): the pairing page, which reads the QR code a device shows with the computer's
//! camera, and the app page (status and devices). Everything they need is in the binary, so they
//! work without internet access, and they answer only to a request that is really local.

use crate::localguard;
use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub const PAIR_HTML: &str = include_str!("../assets/pair/index.html");
pub const PAIR_JS: &str = include_str!("../assets/pair/pair.js");
pub const PAIRLOGIC_JS: &str = include_str!("../assets/pair/pairlogic.js");
pub const JSQR_JS: &str = include_str!("../assets/pair/jsQR.js");
pub const PAIR_STRINGS: &str = include_str!("../assets/pair/strings.tsv");
pub const APP_HTML: &str = include_str!("../assets/app/index.html");
pub const APP_JS: &str = include_str!("../assets/app/app.js");
pub const APP_STRINGS: &str = include_str!("../assets/app/strings.tsv");

/// Maps a language to its messages.
pub type Strings = BTreeMap<String, BTreeMap<String, String>>;

fn valid_key(k: &str) -> bool {
    let mut c = k.chars();
    c.next().is_some_and(|f| f.is_ascii_alphabetic()) && c.all(|x| x.is_ascii_alphanumeric())
}

/// `ja`, `zh_CN`, `fr`: two or three lower-case letters, optionally `_` and two capitals.
fn valid_lang(l: &str) -> bool {
    let (base, region) = match l.split_once('_') {
        Some((b, r)) => (b, Some(r)),
        None => (l, None),
    };
    (2..=3).contains(&base.len()) && base.bytes().all(|b| b.is_ascii_lowercase()) && region.is_none_or(|r| r.len() == 2 && r.bytes().all(|b| b.is_ascii_uppercase()))
}

/// The names of the `{placeholders}` of a text, sorted: two texts agree if these are the same.
fn placeholders(s: &str) -> String {
    let mut out = vec![];
    let mut rest = s;
    while let Some(i) = rest.find('{') {
        let after = &rest[i + 1..];
        match after.find('}') {
            Some(j) if j > 0 && after[..j].bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') => {
                out.push(after[..j].to_string());
                rest = &after[j + 1..];
            }
            _ => rest = after,
        }
    }
    out.sort();
    out.join(",")
}

fn unescape(cell: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut it = cell.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            None => return Err("a cell ends with a backslash".into()),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some(o) => return Err(format!("unknown escape \\{o}")),
        }
    }
    Ok(out)
}

/// Reads a page's words: a TSV with `key<TAB>en<TAB>…` as its header (the same rules as
/// plugin/locales/strings.tsv). An empty cell outside English uses the English text.
pub fn parse_strings(tsv: &str) -> Result<Strings, String> {
    let mut langs: Option<Vec<String>> = None;
    let mut out = Strings::new();
    let mut seen = BTreeSet::new();
    for (n, raw) in tsv.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cells: Vec<&str> = line.split('\t').collect();
        let wher = format!("line {}", n + 1);
        let Some(ls) = &langs else {
            if cells.len() < 2 || cells[0] != "key" || cells[1] != "en" {
                return Err(format!("{wher}: the header must start with key<TAB>en"));
            }
            let mut dup = BTreeSet::new();
            for l in &cells[1..] {
                if !valid_lang(l) {
                    return Err(format!("{wher}: {l:?} is not a language code"));
                }
                if !dup.insert(*l) {
                    return Err(format!("{wher}: language {l} appears twice"));
                }
                out.insert(l.to_string(), BTreeMap::new());
            }
            langs = Some(cells[1..].iter().map(|s| s.to_string()).collect());
            continue;
        };
        if cells.len() != ls.len() + 1 {
            return Err(format!("{wher}: {} columns, expected {}", cells.len(), ls.len() + 1));
        }
        let key = cells[0];
        if !valid_key(key) {
            return Err(format!("{wher}: {key:?} is not a valid key"));
        }
        if !seen.insert(key.to_string()) {
            return Err(format!("{wher}: key {key:?} appears twice"));
        }
        let mut texts = vec![];
        for (i, c) in cells[1..].iter().enumerate() {
            texts.push(unescape(c).map_err(|e| format!("{wher}, {}: {e}", ls[i]))?);
        }
        if texts[0].is_empty() {
            return Err(format!("key {key:?}: the English text is required"));
        }
        for (i, l) in ls.iter().enumerate() {
            if texts[i].is_empty() {
                texts[i] = texts[0].clone();
            }
            if placeholders(&texts[i]) != placeholders(&texts[0]) {
                return Err(format!("key {key:?}, {l}: placeholders {{{}}} differ from English {{{}}}", placeholders(&texts[i]), placeholders(&texts[0])));
            }
            out.get_mut(l).expect("a declared language").insert(key.to_string(), texts[i].clone());
        }
    }
    if langs.is_none() {
        return Err("no header line".into());
    }
    Ok(out)
}

fn file(content_type: &'static str, body: &'static str) -> impl axum::handler::Handler<((),), ()> {
    move || async move { ([(header::CONTENT_TYPE, content_type)], body) }
}

fn json_of(tsv: &str, what: &str) -> Result<String, String> {
    let strs = parse_strings(tsv).map_err(|e| format!("{what} strings.tsv: {e}"))?;
    serde_json::to_string(&strs).map_err(|e| e.to_string())
}

/// Wraps pages so that they answer only to the person at this computer, with the headers that keep
/// them from loading or leaking anything.
fn guarded(pages: Router, csp: &'static str) -> Router {
    pages.layer(middleware::from_fn(move |req: Request<Body>, next: Next| async move {
        if !localguard::local(req.headers(), &localguard::host_of(&req)) {
            return localguard::refuse();
        }
        let mut r: Response = next.run(req).await;
        let h = r.headers_mut();
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
        h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
        h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(csp));
        r
    }))
}

/// The pairing page under /pair. The page has no secret in it; it reads the command line's token
/// from the part of the address after # (which a browser never sends).
pub fn pair_router() -> Result<Router, String> {
    let json: &'static str = Box::leak(json_of(PAIR_STRINGS, "pairpage")?.into_boxed_str());
    let pages = Router::new()
        .route("/pair", get(file("text/html; charset=utf-8", PAIR_HTML)))
        .route("/pair/pair.js", get(file("text/javascript; charset=utf-8", PAIR_JS)))
        .route("/pair/pairlogic.js", get(file("text/javascript; charset=utf-8", PAIRLOGIC_JS)))
        .route("/pair/jsQR.js", get(file("text/javascript; charset=utf-8", JSQR_JS)))
        .route("/pair/strings.json", get(file("application/json; charset=utf-8", json)));
    Ok(guarded(pages, "default-src 'none'; script-src 'self' 'unsafe-inline'; style-src 'unsafe-inline'; media-src 'self' blob:; connect-src 'self'; img-src 'self' data:"))
}

/// The app page under /app (it also needs the pairing page's pairlogic.js, served under /pair).
pub fn app_router() -> Result<Router, String> {
    let json: &'static str = Box::leak(json_of(APP_STRINGS, "appui")?.into_boxed_str());
    let pages = Router::new()
        .route("/app", get(file("text/html; charset=utf-8", APP_HTML)))
        .route("/app/app.js", get(file("text/javascript; charset=utf-8", APP_JS)))
        .route("/app/strings.json", get(file("application/json; charset=utf-8", json)));
    Ok(guarded(pages, "default-src 'none'; script-src 'self' 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; img-src 'self' data:"))
}

/// Both pages, for the bridge's router. A page that cannot be built is left out and the reason is
/// returned (the words are embedded and tested: that is a build mistake, not a runtime condition).
pub fn pages() -> (Router, Vec<String>) {
    let mut app = Router::new();
    let mut problems = vec![];
    match pair_router() {
        Ok(r) => app = app.merge(r),
        Err(e) => problems.push(format!("pairing page disabled: {e}")),
    }
    match app_router() {
        Ok(r) => app = app.merge(r),
        Err(e) => problems.push(format!("app page disabled: {e}")),
    }
    (app, problems)
}

#[allow(dead_code)]
fn unused(_: HashMap<(), ()>, _: StatusCode) -> impl IntoResponse {
    ""
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    const HEADER: &str = "key\ten\tja\n";

    #[test]
    fn parse_strings_reads_languages_as_columns() {
        let s = parse_strings(&format!("# note\n\n{HEADER}hello\tHi {{name}}\\nthere\\there\\\\\tこんにちは {{name}}\r\nbye\tBye\t\n")).unwrap();
        assert_eq!(s["en"]["hello"], "Hi {name}\nthere\there\\");
        assert_eq!(s["ja"]["hello"], "こんにちは {name}");
        assert_eq!(s["ja"]["bye"], "Bye", "an empty cell must use the English text");
        let more = parse_strings("key\ten\tja\tzh_CN\nx\tA\tB\tC\n").unwrap();
        assert_eq!(more["zh_CN"]["x"], "C", "a third language");
    }

    #[test]
    fn parse_strings_refuses_what_is_wrong() {
        let cases: Vec<(&str, String, &str)> = vec![
            ("no header", "x\tA\tB\n".into(), "header"),
            ("nothing at all", String::new(), "no header"),
            ("English not second", "key\tja\ten\n".into(), "header"),
            ("one column header", "key\n".into(), "header"),
            ("a bad language", "key\ten\tJapanese\n".into(), "language code"),
            ("a language twice", "key\ten\tja\tja\n".into(), "twice"),
            ("a missing cell", format!("{HEADER}x\tA\n"), "columns"),
            ("a bad key", format!("{HEADER}1x\tA\tB\n"), "valid key"),
            ("a key twice", format!("{HEADER}x\tA\tB\nx\tA\tB\n"), "twice"),
            ("an unknown escape", format!("{HEADER}x\tA\\q\tB\n"), "escape"),
            ("a trailing backslash", format!("{HEADER}x\tA\\\tB\n"), "backslash"),
            ("no English text", format!("{HEADER}x\t\tB\n"), "English text is required"),
            ("a missing placeholder", format!("{HEADER}x\tHi {{n}}\tこんにちは\n"), "placeholders"),
            ("an extra placeholder", format!("{HEADER}x\tHi\tこんにちは {{n}}\n"), "placeholders"),
        ];
        for (name, input, want) in cases {
            match parse_strings(&input) {
                Err(e) if e.contains(want) => {}
                other => panic!("{name}: {other:?} (want {want:?})"),
            }
        }
    }

    fn complete(tsv: &str, languages: &[&str]) {
        let s = parse_strings(tsv).unwrap();
        for lang in languages {
            assert!(!s[*lang].is_empty(), "no {lang}");
            assert_eq!(s[*lang].len(), s["en"].len(), "{lang} has {} messages, English has {}", s[*lang].len(), s["en"].len());
        }
        // no gap falls back to English silently: every cell of the languages we ship is really there
        let header: Vec<&str> = tsv.lines().find(|l| l.starts_with("key\t")).unwrap().split('\t').collect();
        for line in tsv.lines().filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("key\t")) {
            let cells: Vec<&str> = line.split('\t').collect();
            for (i, lang) in header.iter().enumerate().skip(1) {
                if languages.contains(lang) {
                    assert!(!cells[i].is_empty(), "{} has no {lang} text", cells[0]);
                }
            }
        }
    }

    #[test]
    fn the_embedded_words_are_complete_in_every_language() {
        complete(PAIR_STRINGS, &["en", "ja", "es", "fr"]);
        complete(APP_STRINGS, &["en", "ja", "es", "fr"]);
    }

    fn words(tsv: &str) -> BTreeMap<String, String> {
        parse_strings(tsv).unwrap().remove("en").unwrap()
    }

    /// Every capture group 1 of `re` in `src`.
    fn captures(src: &str, re: &str) -> Vec<String> {
        // a tiny matcher for the few shapes below: `prefix(\w+)suffix` written as prefix\0suffix
        let (pre, suf) = re.split_once('\0').unwrap();
        let mut out = vec![];
        let mut rest = src;
        while let Some(i) = rest.find(pre) {
            let after = &rest[i + pre.len()..];
            let word: String = after.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            // a word boundary before the match, as the Go test's \bt\(
            let boundary = !pre.starts_with(|c: char| c.is_ascii_alphanumeric()) || !rest[..i].ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_');
            if boundary && !word.is_empty() && after[word.len()..].starts_with(suf) {
                out.push(word);
            }
            rest = &rest[i + pre.len()..];
        }
        out
    }

    /// What the page's script asks for must exist in the words, or a person would see a bare key.
    #[test]
    fn every_message_the_pairing_page_uses_exists() {
        let s = words(PAIR_STRINGS);
        let mut used = BTreeSet::new();
        used.extend(captures(PAIR_JS, "say('\0'")); // say('hint')
        for line in PAIR_JS.lines() {
            // ['title', 'x'].forEach
            if let (Some(a), Some(b)) = (line.find("['"), line.find("].forEach")) {
                if a < b {
                    for k in line[a + 1..b].split(',').map(|k| k.trim().trim_matches('\'')) {
                        used.insert(k.to_string());
                    }
                }
            }
        }
        for line in PAIRLOGIC_JS.lines() {
            // case 3: return 'key'
            if let Some(i) = line.find("return '") {
                if line.contains("case ") {
                    used.insert(line[i + 8..].split('\'').next().unwrap().to_string());
                }
            }
        }
        for k in &used {
            assert!(s.contains_key(k), "the page uses the message {k:?}, which strings.tsv does not have");
        }
        assert!(used.len() >= 8, "found only {} messages in use: the check itself is broken", used.len());
    }

    #[test]
    fn every_message_the_app_page_uses_exists() {
        let s = words(APP_STRINGS);
        let mut used = BTreeSet::new();
        for pattern in ["t('\0',", "t('\0')", "say('\0',", "say('\0')", "'done_\0'"] {
            used.extend(captures(APP_JS, pattern));
        }
        used.extend(captures(APP_JS, "confirm\0'").into_iter().map(|k| format!("confirm{k}")));
        used.retain(|k| !k.is_empty());
        // the messages built from a state or an action
        for st in ["issued", "waiting", "granting", "active", "expired", "revoked"] {
            used.insert(format!("state{}{}", st[..1].to_uppercase(), &st[1..]));
        }
        for a in ["Pair", "Save", "Revoke", "Reissue", "Forget"] {
            used.insert(format!("action{a}"));
        }
        for a in ["Revoke", "Reissue", "Forget"] {
            used.insert(format!("done{a}"));
            used.insert(format!("confirm{a}"));
        }
        for k in ["title", "devicesTitle", "addTitle", "addButton", "addPlaceholder"] {
            used.insert(k.to_string()); // set straight from the words
        }
        for k in &used {
            if k.starts_with("done_") || k.starts_with("step") && k.len() <= 6 {
                continue;
            }
            assert!(s.contains_key(k), "the page uses the message {k:?}, which strings.tsv does not have");
        }
        assert!(used.len() >= 25, "found only {} messages in use: the check itself is broken", used.len());
        // and every message in the words is used by something (no dead words)
        for k in s.keys() {
            assert!(
                used.contains(k) || APP_JS.contains(&format!("'{k}'")) || APP_JS.contains(&format!("t('{k}'")) || APP_JS.contains(&format!("strings.{k}")) || APP_JS.contains(k.as_str()),
                "the message {k:?} is in strings.tsv but the page never uses it"
            );
        }
    }

    async fn get(router: Router, path: &str, host: &str, extra: &[(&str, &str)]) -> (StatusCode, axum::http::HeaderMap, String) {
        let mut b = axum::http::Request::builder().uri(path).header("host", host);
        for (k, v) in extra {
            b = b.header(*k, *v);
        }
        let resp = router.oneshot(b.body(Body::empty()).unwrap()).await.unwrap();
        let (status, headers) = (resp.status(), resp.headers().clone());
        let body = axum::body::to_bytes(resp.into_body(), 1 << 22).await.unwrap();
        (status, headers, String::from_utf8_lossy(&body).into_owned())
    }

    #[tokio::test]
    async fn the_pairing_page_and_its_parts_are_served_with_the_security_headers() {
        for (path, want) in [("/pair", "text/html"), ("/pair/pair.js", "text/javascript"), ("/pair/pairlogic.js", "text/javascript"), ("/pair/jsQR.js", "text/javascript"), ("/pair/strings.json", "application/json")] {
            let (status, h, body) = get(pair_router().unwrap(), path, "127.0.0.1:8765", &[]).await;
            assert!(status == 200 && h["content-type"].to_str().unwrap().starts_with(want) && !body.is_empty(), "{path}: {status} {:?}", h.get("content-type"));
            assert!(
                h["cache-control"] == "no-store" && h["referrer-policy"] == "no-referrer" && h["content-security-policy"].to_str().unwrap().contains("default-src 'none'") && h["x-content-type-options"] == "nosniff",
                "{path}: security headers missing: {h:?}"
            );
        }
        assert_eq!(get(pair_router().unwrap(), "/pair/nothing.js", "127.0.0.1:8765", &[]).await.0, 404, "an unknown file");
    }

    #[tokio::test]
    async fn the_app_page_and_its_parts_are_served_with_the_security_headers() {
        for (path, want) in [("/app", "text/html"), ("/app/app.js", "text/javascript"), ("/app/strings.json", "application/json")] {
            let (status, h, body) = get(app_router().unwrap(), path, "127.0.0.1:8765", &[]).await;
            assert!(status == 200 && h["content-type"].to_str().unwrap().starts_with(want) && !body.is_empty(), "{path}: {status}");
            assert!(h["cache-control"] == "no-store" && h["content-security-policy"].to_str().unwrap().contains("default-src 'none'"), "{path}: {h:?}");
        }
        assert_eq!(get(app_router().unwrap(), "/app/other.js", "127.0.0.1:8765", &[]).await.0, 404);
        let (_, _, body) = get(app_router().unwrap(), "/app/strings.json", "localhost", &[]).await;
        let got: BTreeMap<String, BTreeMap<String, String>> = serde_json::from_str(&body).unwrap();
        assert!(!got["ja"]["actionRevoke"].is_empty());
    }

    #[tokio::test]
    async fn the_strings_are_served_as_json_for_every_language() {
        let (_, _, body) = get(pair_router().unwrap(), "/pair/strings.json", "127.0.0.1:1", &[]).await;
        let got: BTreeMap<String, BTreeMap<String, String>> = serde_json::from_str(&body).unwrap();
        assert!(!got["en"]["title"].is_empty() && !got["ja"]["title"].is_empty() && got["ja"]["title"] != got["en"]["title"], "{} / {}", got["en"]["title"], got["ja"]["title"]);
        for lang in ["es", "fr"] {
            assert!(got.contains_key(lang) && got[lang]["title"] != got["en"]["title"], "{lang}");
        }
    }

    #[tokio::test]
    async fn the_pages_are_not_for_a_web_page_that_pointed_its_own_name_here() {
        for host in ["evil.example:8765", "192.168.2.180:8765", "bridge.spellink.uk"] {
            assert_eq!(get(pair_router().unwrap(), "/pair", host, &[]).await.0, 403, "Host {host}");
            assert_eq!(get(app_router().unwrap(), "/app", host, &[]).await.0, 403, "Host {host}");
        }
    }

    #[tokio::test]
    async fn the_pages_are_not_for_anyone_behind_a_proxy() {
        for h in ["Cf-Connecting-Ip", "X-Forwarded-For"] {
            assert_eq!(get(pair_router().unwrap(), "/pair", "127.0.0.1:8765", &[(h, "203.0.113.9")]).await.0, 403, "{h}");
            assert_eq!(get(app_router().unwrap(), "/app", "127.0.0.1:8765", &[(h, "203.0.113.9")]).await.0, 403, "{h}");
        }
    }

    #[tokio::test]
    async fn the_pages_have_no_secret_and_no_outside_script() {
        for (html, prefixes) in [(get(pair_router().unwrap(), "/pair", "127.0.0.1", &[]).await.2, vec!["/pair/"]), (get(app_router().unwrap(), "/app", "127.0.0.1", &[]).await.2, vec!["/app/", "/pair/pairlogic.js"])] {
            assert!(!html.contains("http://") && !html.contains("https://"), "the page must not load anything from outside (it works offline, and nothing outside may run in it)");
            for src in captures(&html, "src=\"\0\"").into_iter().chain(html.split("src=\"").skip(1).map(|r| r.split('"').next().unwrap().to_string())) {
                assert!(prefixes.iter().any(|p| src.starts_with(p)), "script {src} is not served by the bridge");
            }
        }
    }
}

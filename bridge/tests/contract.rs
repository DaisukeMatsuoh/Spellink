//! The frames the bridge really sends, from a real session of every kind, against
//! `testdata/frames.golden.json`; the plugin's tests (plugin/__tests__/contract.test.ts) check that
//! its validator accepts every one of them. If this fails after a deliberate change to a frame:
//! run `UPDATE_GOLDEN=1 cargo test --test contract`, then the plugin's tests.
//! (The golden file was recorded by the Go bridge, which this one replaced: the same shapes.)

mod common;

use common::*;
use serde_json::{Value, json};
use spellink_bridge::provider::AskError;
use std::collections::BTreeMap;
use std::time::Duration;

const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/frames.golden.json");

/// Names a frame by its type, its stage or code, and the fields it carries: the plugin's validator
/// (plugin/src/frames.ts) depends on exactly these.
fn shape(m: &Value) -> String {
    let obj = m.as_object().unwrap();
    let mut keys: Vec<&String> = obj.keys().collect();
    keys.sort();
    let mut kind = m["type"].as_str().unwrap().to_string();
    if let Some(s) = obj.get("stage").and_then(Value::as_str) {
        kind += &format!(":{s}");
    }
    if let Some(c) = obj.get("code").and_then(Value::as_str) {
        kind += &format!(":{c}");
    }
    if let (Some(p), Some("alive")) = (obj.get("phase").and_then(Value::as_str), obj.get("stage").and_then(Value::as_str)) {
        kind += &format!(":{p}");
    }
    format!("{kind} {{{}}}", keys.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(","))
}

#[tokio::test]
async fn the_frames_the_bridge_sends_are_the_ones_the_plugin_expects() {
    let url = start_with(
        ai(|q, _, s, c| async move {
            if q.contains("cancel-me") {
                // says nothing at all until it is cancelled
                c.cancelled().await;
                return Err(AskError::Cancelled(String::new()));
            }
            text(&s, "あああ\n");
            tokio::time::sleep(Duration::from_millis(80)).await; // long enough for keep-alives while writing
            text(&s, "あ");
            Ok("あああ\nあ".to_string())
        }),
        |d| d.keepalive_every = Duration::from_millis(20),
    )
    .await;
    let quiet = Duration::from_millis(300);
    let mut frames: Vec<Value> = vec![];
    // a full answer: welcome, status, alive, line, done
    let mut c = Client::dial(&url).await;
    c.send(ask("g1", "/n/a.note", "normal")).await;
    frames.extend(c.until_quiet(quiet).await);
    // a cancelled answer that never started: keep-alives while starting, then the ending error
    let mut c2 = Client::dial(&url).await;
    c2.send(ask("g2", "/n/b.note", "cancel-me")).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    c2.send(json!({"type": "cancel", "id": "g2"})).await;
    frames.extend(c2.until_quiet(quiet).await);
    // an answer that is not kept, settings, the memory
    let mut c3 = Client::dial(&url).await;
    c3.send(json!({"type": "resume", "id": "nobody", "next": 0})).await;
    c3.send(json!({"type": "setSettings", "settings": {"showProgress": true}})).await;
    c3.send(json!({"type": "clearHistory"})).await;
    frames.extend(c3.until_quiet(quiet).await);
    // the welcome of a fresh connection
    let mut w = Client::connect(&url).await;
    w.send(hello()).await;
    frames.extend(w.recv().await);

    let mut seen: BTreeMap<String, Value> = BTreeMap::new();
    for f in &frames {
        if f["type"].is_string() {
            seen.entry(shape(f)).or_insert_with(|| f.clone());
        }
    }
    let shapes: Vec<&String> = seen.keys().collect();
    for want in ["welcome", "status:thinking", "status:writing", "status:alive:starting", "line", "done", "error:cancelled", "error:unknown_ask", "settings", "historyCleared"] {
        assert!(shapes.iter().any(|s| s.starts_with(&format!("{want} ")) || s.starts_with(&format!("{want}:"))), "the session produced no {want:?} frame; got {shapes:?}");
    }
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        let examples: Vec<&Value> = seen.values().collect();
        std::fs::write(GOLDEN, serde_json::to_string_pretty(&examples).unwrap() + "\n").unwrap();
        return;
    }
    let golden: Vec<Value> = serde_json::from_slice(&std::fs::read(GOLDEN).expect("no golden file (run with UPDATE_GOLDEN=1)")).unwrap();
    let golden_shapes: std::collections::BTreeSet<String> = golden.iter().map(shape).collect();
    for s in &shapes {
        assert!(golden_shapes.contains(*s), "a frame the golden file does not have: {s} (run with UPDATE_GOLDEN=1, then the plugin tests)");
    }
    for s in &golden_shapes {
        assert!(seen.contains_key(s), "the golden file has a frame the bridge no longer sends: {s}");
    }
}

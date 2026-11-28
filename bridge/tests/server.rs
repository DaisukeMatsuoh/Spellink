//! The bridge's session, through its real entry point: a WebSocket on a port, the way the plugin
//! talks to it (Go: server_test.go).

mod common;

use common::*;
use serde_json::{Value, json};
use spellink_bridge::history::{self, Turn};
use spellink_bridge::layout::Layout;
use spellink_bridge::provider::AskError;
use spellink_bridge::server::session::valid_layout;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn timeout_deps(first: Duration, gap: Duration, total: Duration) -> impl FnOnce(&mut spellink_bridge::server::Deps) {
    move |d| {
        d.first_output = first;
        d.gap = gap;
        d.total = total;
    }
}

#[tokio::test]
async fn it_rejects_a_wrong_token() {
    let url = start(ai(|_, _, _, _| async { Ok(String::new()) })).await;
    let got = converse(&url, vec![json!({"type": "hello", "token": "nope"})], |_| false).await;
    assert_eq!(types(&got), "error");
    assert_eq!(got[0]["code"], "unauthorized");
}

#[tokio::test]
async fn it_streams_lines_and_finishes() {
    let prompt = Arc::new(Mutex::new(String::new()));
    let p2 = prompt.clone();
    let url = start(ai(move |_, sys, s, _| {
        let p = p2.clone();
        async move {
            *p.lock().unwrap() = sys;
            text(&s, "あああ");
            text(&s, "\nあ");
            Ok("あああ\nあ".to_string())
        }
    }))
    .await;
    let got = converse(&url, vec![hello(), json!({"type": "ask", "id": "q1", "question": "test?", "lang": "ja", "layout": layout()})], finished).await;
    assert_eq!(types(&got), "welcome,status,status,line,line,done");
    assert!(got[3]["text"] == "あああ" && got[3]["strokes"].as_array().unwrap().len() == 3 && got[3]["page"] == 0, "first line: {}", got[3]);
    assert_eq!(got[5]["pages"], 1, "done: {}", got[5]);
    assert!(prompt.lock().unwrap().contains("字以内"), "system prompt lacks a length budget");
}

#[tokio::test]
async fn a_follow_up_in_the_same_note_sees_the_earlier_conversation() {
    let asked = Arc::new(Mutex::new(Vec::<String>::new()));
    let a2 = asked.clone();
    let dir = tempfile::tempdir().unwrap();
    let url = start_with(
        ai(move |q, _, s, _| {
            let asked = a2.clone();
            async move {
                asked.lock().unwrap().push(q.clone());
                text(&s, "あ");
                Ok(format!("答え:{q}"))
            }
        }),
        |d| {
            d.history = Arc::new(history::Store::new(dir.path().join("h")));
            d.history_chars = 1000;
        },
    )
    .await;
    // separate connections: the memory belongs to the note, not to the socket
    converse(&url, vec![hello(), ask("1", "/n/a.note", "1+1は？")], finished).await;
    converse(&url, vec![hello(), ask("2", "/n/a.note", "それを3倍して")], finished).await;
    converse(&url, vec![hello(), ask("3", "/n/other.note", "別の話")], finished).await;
    let asked = asked.lock().unwrap().clone();
    assert_eq!(asked[0], "1+1は？", "a first question is sent as is");
    for want in ["1+1は？", "答え:1+1は？", "それを3倍して"] {
        assert!(asked[1].contains(want), "follow-up lacks {want:?}: {:?}", asked[1]);
    }
    assert_eq!(asked[2], "別の話", "another note must not see it");
}

#[tokio::test]
async fn failed_or_cancelled_questions_are_not_remembered() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(history::Store::new(dir.path().join("h")));
    let s2 = store.clone();
    let url = start_with(ai(|_, _, _, _| async { Err(AskError::Failed("boom".into())) }), move |d| {
        d.history = s2;
        d.history_chars = 1000;
    })
    .await;
    converse(&url, vec![hello(), ask("1", "n", "q")], finished).await;
    assert!(store.recent("n", 1000).unwrap().is_empty(), "remembered a failed question");
}

#[tokio::test]
async fn a_note_without_a_key_or_with_the_memory_off_is_stateless() {
    let asked = Arc::new(Mutex::new(Vec::<String>::new()));
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(history::Store::new(dir.path().join("h")));
    let mk = |asked: Arc<Mutex<Vec<String>>>| {
        ai(move |q, _, s, _| {
            let asked = asked.clone();
            async move {
                asked.lock().unwrap().push(q);
                text(&s, "あ");
                Ok("a".to_string())
            }
        })
    };
    let st = store.clone();
    let on = start_with(mk(asked.clone()), move |d| {
        d.history = st;
        d.history_chars = 1000;
    })
    .await;
    converse(&on, vec![hello(), ask("1", "", "q1")], finished).await;
    converse(&on, vec![hello(), ask("2", "", "q2")], finished).await;
    let st = store.clone();
    let off = start_with(mk(asked.clone()), move |d| {
        d.history = st;
        d.history_chars = 0;
    })
    .await;
    converse(&off, vec![hello(), ask("3", "n", "q3")], finished).await;
    converse(&off, vec![hello(), ask("4", "n", "q4")], finished).await;
    let asked = asked.lock().unwrap().clone();
    assert!(asked[1] == "q2" && asked[3] == "q4", "expected stateless questions, got {asked:?}");
}

fn seeded_store(dir: &std::path::Path, notes: &[&str]) -> Arc<history::Store> {
    let s = Arc::new(history::Store::new(dir.join("h")));
    for n in notes {
        s.append(n, &Turn { at: String::new(), q: "q".into(), a: "a".into() }).unwrap();
    }
    s
}

#[tokio::test]
async fn clear_history_forgets_all_notes() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded_store(dir.path(), &["a", "b"]);
    let st = store.clone();
    let url = start_with(ai(|_, _, _, _| async { Ok(String::new()) }), move |d| {
        d.history = st;
        d.history_chars = 1000;
    })
    .await;
    let got = converse(&url, vec![hello(), json!({"type": "clearHistory"})], |m| m["type"] == "historyCleared").await;
    assert_eq!(got[1]["notes"], 2, "{got:?}");
    assert!(store.recent("a", 1000).unwrap().is_empty());
}

#[tokio::test]
async fn it_reports_the_ai_failure() {
    let url = start(ai(|_, _, _, _| async { Err(AskError::Failed("not logged in".into())) })).await;
    let got = converse(&url, vec![hello(), json!({"type": "ask", "id": "q2", "question": "x", "lang": "en", "layout": layout()})], |m| m["type"] == "error").await;
    assert_eq!(got.last().unwrap()["code"], "provider_failed");
}

#[tokio::test]
async fn cancel_stops_the_model() {
    let stopped = Arc::new(Notify::new());
    let st = stopped.clone();
    let url = start(ai(move |_, _, _, c| {
        let stopped = st.clone();
        async move {
            c.cancelled().await;
            stopped.notify_one();
            Err(AskError::Cancelled(String::new()))
        }
    }))
    .await;
    let got = converse(&url, vec![hello(), json!({"type": "ask", "id": "q3", "question": "x", "lang": "ja", "layout": layout()}), json!({"type": "cancel", "id": "q3"})], |m| m["type"] == "error").await;
    tokio::time::timeout(Duration::from_secs(5), stopped.notified()).await.expect("the model was told to stop");
    assert_eq!(got.last().unwrap()["code"], "cancelled");
}

#[test]
fn layout_validation() {
    let ok: Layout = serde_json::from_value(layout()).unwrap();
    assert!(valid_layout(&ok), "valid layout rejected");
    let mutations: Vec<(&str, Box<dyn Fn(&mut Layout)>)> = vec![
        ("charSize", Box::new(|l| l.char_size = 0.0)),
        ("y", Box::new(|l| l.y = 5000.0)),
        ("maxPages", Box::new(|l| l.max_pages = 99)),
        ("top", Box::new(|l| l.continue_top = -1.0)),
        ("limit", Box::new(|l| l.start_limit = 99999.0)),
        ("nan", Box::new(|l| l.width = f64::NAN)),
        ("inf", Box::new(|l| l.x = f64::INFINITY)),
        ("negative pages", Box::new(|l| l.max_pages = -1)),
    ];
    for (name, mutate) in mutations {
        let mut l = ok.clone();
        mutate(&mut l);
        assert!(!valid_layout(&l), "{name}: invalid layout accepted");
    }
}

#[tokio::test]
async fn proxied_connections_must_be_encrypted() {
    let url = start(ai(|_, _, _, _| async { Ok(String::new()) })).await;
    let mut c = Client::connect_with_header(&url, "cf-connecting-ip", "203.0.113.9").await;
    c.send(hello()).await;
    let reply = c.recv().await.expect("a reply");
    assert!(reply.to_string().contains("unauthorized"), "plaintext hello through a proxy should be refused, got {reply}");
}

fn settings_server(stored: Arc<Mutex<spellink_bridge::protocol::Settings>>) -> impl FnOnce(&mut spellink_bridge::server::Deps) {
    move |d| {
        let (a, b) = (stored.clone(), stored);
        d.settings = Arc::new(move || a.lock().unwrap().clone());
        d.set_settings = Arc::new(move |s| {
            *b.lock().unwrap() = s;
            Ok(())
        });
    }
}

#[tokio::test]
async fn settings_round_trip() {
    let stored = Arc::new(Mutex::new(Default::default()));
    let url = start_with(ai(|_, _, _, _| async { Ok(String::new()) }), settings_server(stored.clone())).await;
    let got = converse(&url, vec![hello(), json!({"type": "setSettings", "settings": {"showProgress": true}})], |m| m["type"] == "settings").await;
    assert_eq!(types(&got), "welcome,settings");
    assert_eq!(got[0]["settings"]["showProgress"], false, "welcome should carry the old settings");
    assert!(got[1]["settings"]["showProgress"] == true && stored.lock().unwrap().show_progress, "setting not applied: {}", got[1]);
}

#[tokio::test]
async fn settings_are_persisted_together() {
    let stored = Arc::new(Mutex::new(spellink_bridge::protocol::Settings { full_width_spacing: 100, half_width_spacing: 110, thinking_animation: true, ..Default::default() }));
    let url = start_with(ai(|_, _, _, _| async { Ok(String::new()) }), settings_server(stored)).await;
    let got = converse(&url, vec![hello(), json!({"type": "setSettings", "settings": {"thinkingAnimation": false, "fullWidthSpacing": 120, "halfWidthSpacing": 130}})], |m| m["type"] == "settings").await;
    let s = &got.last().unwrap()["settings"];
    assert!(s["thinkingAnimation"] == false && s["fullWidthSpacing"] == 120 && s["halfWidthSpacing"] == 130, "{s}");
}

#[tokio::test]
async fn a_setting_message_without_settings_is_refused() {
    let url = start(ai(|_, _, _, _| async { Ok(String::new()) })).await;
    let got = converse(&url, vec![hello(), json!({"type": "setSettings"})], |m| m["type"] == "error").await;
    assert_eq!(got.last().unwrap()["code"], "bad_request");
}

#[tokio::test]
async fn an_unknown_message_type_is_refused_and_ends_the_session() {
    let url = start(ai(|_, _, _, _| async { Ok(String::new()) })).await;
    let mut c = Client::dial(&url).await;
    c.send(json!({"type": "dance"})).await;
    let m = c.recv().await.unwrap();
    assert!(m["type"] == "error" && m["code"] == "bad_request", "{m}");
    assert!(c.recv().await.is_none(), "the session ends");
}

#[tokio::test]
async fn a_frame_that_is_not_json_is_refused() {
    use futures_util::SinkExt;
    let url = start(ai(|_, _, _, _| async { Ok(String::new()) })).await;
    let mut c = Client::dial(&url).await;
    c.ws.send(tokio_tungstenite::tungstenite::Message::Text("{not json".into())).await.unwrap();
    let m = c.recv().await.unwrap();
    assert!(m["type"] == "error" && m["code"] == "bad_request", "{m}");
}

#[tokio::test]
async fn an_invalid_ask_is_refused_without_ending_the_session() {
    let started = Arc::new(Mutex::new(0));
    let s2 = started.clone();
    let url = start(ai(move |_, _, _, _| {
        let s = s2.clone();
        async move {
            *s.lock().unwrap() += 1;
            Ok(String::new())
        }
    }))
    .await;
    let mut c = Client::dial(&url).await;
    let mut bad_layout = ask("x", "", "q");
    bad_layout["layout"]["charSize"] = json!(0);
    for bad in [bad_layout, json!({"type": "ask", "id": "x", "question": "q", "lang": "ja"}), ask("", "", "q"), ask("x", "", "   "), ask("x", "", &"あ".repeat(4001))] {
        c.send(bad).await;
        let m = c.recv().await.unwrap();
        assert!(m["type"] == "error" && m["code"] == "bad_request" && m["message"] == "invalid ask", "{m}");
    }
    assert_eq!(*started.lock().unwrap(), 0, "no invalid question reaches the AI");
    c.send(ask("ok", "", "q")).await;
    assert_eq!(c.last_frame().await["type"], "done", "the session still works");
}

// ---- reconnecting ----

#[tokio::test]
async fn a_dropped_connection_can_fetch_the_rest_of_the_answer() {
    let release = Arc::new(Notify::new());
    let r2 = release.clone();
    let url = start(ai(move |_, _, s, _| {
        let release = r2.clone();
        async move {
            text(&s, "あああ\n"); // one whole line before the connection drops
            release.notified().await;
            text(&s, "あ\nあ");
            Ok("あああ\nあ\nあ".to_string())
        }
    }))
    .await;
    let mut c = Client::dial(&url).await;
    c.send(ask("a1", "", "q")).await;
    let first = loop {
        let m = c.recv().await.unwrap();
        if m["type"] == "line" {
            break m;
        }
    };
    assert_eq!(first["seq"], 0, "first frame must be number 0: {first}");
    drop(c); // the tunnel drops while the AI is still answering
    release.notify_one();

    let mut c2 = Client::dial(&url).await;
    c2.send(json!({"type": "resume", "id": "a1", "next": 1})).await;
    let mut seqs = vec![];
    loop {
        let m = c2.recv().await.unwrap();
        if m["type"] == "status" {
            continue;
        }
        seqs.push(m["seq"].as_i64().unwrap());
        assert_ne!(m["type"], "error", "unexpected error: {m}");
        if m["type"] == "done" {
            break;
        }
    }
    assert!(seqs.len() >= 2 && seqs[0] == 1, "must continue at frame 1 without repeating frame 0, got {seqs:?}");
    assert!(seqs.windows(2).all(|w| w[1] == w[0] + 1), "frames must be consecutive: {seqs:?}");
}

#[tokio::test]
async fn resuming_an_unknown_answer_says_so() {
    let url = start(ai(|_, _, _, _| async { Ok(String::new()) })).await;
    let mut c = Client::dial(&url).await;
    c.send(json!({"type": "resume", "id": "never", "next": 0})).await;
    let m = c.recv().await.unwrap();
    assert!(m["type"] == "error" && m["code"] == "unknown_ask", "{m}");
}

#[tokio::test]
async fn an_answer_nobody_comes_back_for_is_cancelled() {
    let stopped = Arc::new(Notify::new());
    let st = stopped.clone();
    let url = start_with(
        ai(move |_, _, s, c| {
            let stopped = st.clone();
            async move {
                text(&s, "あ");
                c.cancelled().await;
                stopped.notify_one();
                Err(AskError::Cancelled(String::new()))
            }
        }),
        |d| d.orphan_grace = ms(50),
    )
    .await;
    let mut c = Client::dial(&url).await;
    c.send(ask("o1", "", "q")).await;
    loop {
        let m = c.recv().await.unwrap();
        if m["type"] == "status" && m["stage"] == "writing" {
            break;
        }
    }
    drop(c);
    tokio::time::timeout(Duration::from_secs(3), stopped.notified()).await.expect("an answer with no plugin attached must stop spending the AI's usage");
}

#[tokio::test]
async fn resuming_before_the_grace_ends_keeps_the_answer_alive() {
    let release = Arc::new(Notify::new());
    let r2 = release.clone();
    let url = start_with(
        ai(move |_, _, s, c| {
            let release = r2.clone();
            async move {
                text(&s, "あ");
                tokio::select! {
                    _ = release.notified() => Ok("あ".to_string()),
                    _ = c.cancelled() => Err(AskError::Cancelled(String::new())),
                }
            }
        }),
        |d| d.orphan_grace = ms(300),
    )
    .await;
    let mut c = Client::dial(&url).await;
    c.send(ask("k1", "", "q")).await;
    loop {
        let m = c.recv().await.unwrap();
        if m["type"] == "status" && m["stage"] == "writing" {
            break;
        }
    }
    drop(c);
    let mut c2 = Client::dial(&url).await;
    c2.send(json!({"type": "resume", "id": "k1", "next": 0})).await;
    tokio::time::sleep(ms(600)).await; // well past the grace
    release.notify_one();
    loop {
        let m = c2.recv().await.expect("the answer ended");
        assert_ne!(m["type"], "error", "the resumed answer was cancelled: {m}");
        if m["type"] == "done" {
            return;
        }
    }
}

// ---- the AI's silence ----

#[tokio::test]
async fn an_ai_that_never_starts_is_stopped() {
    let url = start_with(
        ai(|_, _, _, c| async move {
            c.cancelled().await;
            Err(AskError::Cancelled(String::new()))
        }),
        timeout_deps(ms(60), Duration::from_secs(1), Duration::from_secs(5)),
    )
    .await;
    let mut c = Client::dial(&url).await;
    c.send(ask("t1", "", "q")).await;
    let m = c.last_frame().await;
    assert!(m["type"] == "error" && m["code"] == "provider_timeout" && m["message"].as_str().unwrap().contains("no sign"), "{m}");
    assert!(m.get("seq").is_some(), "the error ends the answer, so it carries a sequence number: {m}");
}

#[tokio::test]
async fn an_ai_that_stops_mid_answer_is_stopped() {
    let url = start_with(
        ai(|_, _, s, c| async move {
            text(&s, "あ");
            c.cancelled().await;
            Err(AskError::Cancelled(String::new()))
        }),
        timeout_deps(Duration::from_secs(1), ms(60), Duration::from_secs(5)),
    )
    .await;
    let mut c = Client::dial(&url).await;
    c.send(ask("t2", "", "q")).await;
    let m = c.last_frame().await;
    assert!(m["code"] == "provider_timeout" && m["message"].as_str().unwrap().contains("stopped working"), "{m}");
}

#[tokio::test]
async fn a_total_limit_stops_an_ai_that_keeps_trickling() {
    let url = start_with(
        ai(|_, _, s, c| async move {
            loop {
                tokio::select! {
                    _ = c.cancelled() => return Err(AskError::Cancelled(String::new())),
                    _ = tokio::time::sleep(ms(10)) => text(&s, "あ"), // never silent for long, never finishes
                }
            }
        }),
        timeout_deps(Duration::from_secs(1), Duration::from_secs(1), ms(150)),
    )
    .await;
    let mut c = Client::dial(&url).await;
    c.send(ask("t3", "", "q")).await;
    let m = c.last_frame().await;
    assert!(m["code"] == "provider_timeout" && m["message"].as_str().unwrap().contains("finish"), "{m}");
}

#[tokio::test]
async fn a_steady_answer_within_the_limits_is_not_cut() {
    let url = start_with(
        ai(|_, _, s, _| async move {
            for _ in 0..5 {
                tokio::time::sleep(ms(30)).await;
                text(&s, "あ");
            }
            Ok("あああああ".to_string())
        }),
        timeout_deps(ms(500), ms(200), Duration::from_secs(2)),
    )
    .await;
    let mut c = Client::dial(&url).await;
    c.send(ask("t4", "", "q")).await;
    let m = c.last_frame().await;
    assert_eq!(m["type"], "done", "a normal answer must complete: {m}");
}

#[tokio::test]
async fn a_running_answer_says_it_is_alive() {
    let url = start_with(
        ai(|_, _, _, c| async move {
            tokio::select! {
                _ = tokio::time::sleep(ms(300)) => Ok(String::new()),
                _ = c.cancelled() => Err(AskError::Cancelled(String::new())),
            }
        }),
        |d| d.keepalive_every = ms(30),
    )
    .await;
    let mut c = Client::dial(&url).await;
    c.send(ask("t5", "", "q")).await;
    let mut alive = 0;
    loop {
        let m = c.recv().await.unwrap();
        if finished(&m) {
            break;
        }
        if m["type"] == "status" && m["stage"] == "alive" {
            alive += 1;
        }
    }
    assert!(alive >= 3, "expected several keep-alives while the AI is silent, got {alive}");
}

#[tokio::test]
async fn thinking_without_text_is_not_silence() {
    let url = start_with(
        ai(|_, _, s, _| async move {
            (s.activity)(false); // the process started
            for _ in 0..10 {
                tokio::time::sleep(ms(25)).await;
                (s.activity)(true); // the model is working, but there is no text yet (250 ms > the 80 ms limit)
            }
            text(&s, "あ");
            Ok("あ".to_string())
        }),
        timeout_deps(ms(200), ms(80), Duration::from_secs(5)),
    )
    .await;
    let mut c = Client::dial(&url).await;
    c.send(ask("th1", "", "q")).await;
    let m = c.last_frame().await;
    assert_eq!(m["type"], "done", "an AI that is thinking must not be cut: {m}");
}

#[tokio::test]
async fn a_process_whose_model_never_begins_is_stopped() {
    let url = start_with(
        ai(|_, _, s, c| async move {
            loop {
                tokio::select! {
                    _ = c.cancelled() => return Err(AskError::Cancelled(String::new())),
                    _ = tokio::time::sleep(ms(20)) => (s.activity)(false), // alive, but the model never answers
                }
            }
        }),
        timeout_deps(ms(100), Duration::from_secs(1), Duration::from_secs(5)),
    )
    .await;
    let mut c = Client::dial(&url).await;
    c.send(ask("th2", "", "q")).await;
    let m = c.last_frame().await;
    assert!(m["code"] == "provider_timeout" && m["message"].as_str().unwrap().contains("no sign"), "{m}");
}

#[tokio::test]
async fn a_model_that_went_quiet_after_working_is_stopped() {
    let url = start_with(
        ai(|_, _, s, c| async move {
            (s.activity)(true);
            c.cancelled().await;
            Err(AskError::Cancelled(String::new()))
        }),
        timeout_deps(Duration::from_secs(1), ms(70), Duration::from_secs(5)),
    )
    .await;
    let mut c = Client::dial(&url).await;
    c.send(ask("th3", "", "q")).await;
    let m = c.last_frame().await;
    assert!(m["code"] == "provider_timeout" && m["message"].as_str().unwrap().contains("stopped working"), "{m}");
}

#[tokio::test]
async fn the_keep_alive_says_where_the_ai_is() {
    let url = start_with(
        ai(|_, _, s, _| async move {
            tokio::time::sleep(ms(150)).await; // starting: nothing yet
            (s.activity)(true);
            tokio::time::sleep(ms(150)).await; // working: no text
            text(&s, "あ");
            Ok("あ".to_string())
        }),
        |d| d.keepalive_every = ms(30),
    )
    .await;
    let mut c = Client::dial(&url).await;
    c.send(ask("th4", "", "q")).await;
    let mut phases = std::collections::HashSet::new();
    loop {
        let m = c.recv().await.unwrap();
        if finished(&m) {
            break;
        }
        if m["type"] == "status" && m["stage"] == "alive" {
            assert!(m.get("idleMs").is_some(), "idleMs should accompany every keep-alive: {m}");
            phases.insert(m["phase"].as_str().unwrap().to_string());
        }
    }
    assert!(phases.contains("starting") && phases.contains("working"), "keep-alives should report starting and then working, got {phases:?}");
}

// ---- endings ----

fn terminals(frames: &[Value]) -> (usize, String) {
    let mut n = 0;
    let mut last = String::new();
    for m in frames {
        if m["type"] == "done" || (m["type"] == "error" && !m["seq"].is_null()) {
            n += 1;
            last = format!("{}:{}", m["type"].as_str().unwrap(), m.get("code").map(|c| c.as_str().unwrap_or("")).unwrap_or(""));
        }
    }
    (n, last)
}

#[tokio::test]
async fn a_late_result_after_a_cancel_is_ignored() {
    let release = Arc::new(Notify::new());
    let r2 = release.clone();
    let url = start(ai(move |_, _, s, _| {
        let release = r2.clone();
        async move {
            text(&s, "あ\n");
            release.notified().await;
            text(&s, "い"); // after the cancel: must never reach the plugin
            Ok("あい".to_string()) // …and a successful finish must not add a second ending
        }
    }))
    .await;
    let mut c = Client::dial(&url).await;
    c.send(ask("lc1", "", "q")).await;
    loop {
        if c.recv().await.unwrap()["type"] == "line" {
            break;
        }
    }
    c.send(json!({"type": "cancel", "id": "lc1"})).await;
    loop {
        let m = c.recv().await.unwrap();
        if m["type"] == "error" && m["code"] == "cancelled" {
            break;
        }
    }
    release.notify_one(); // the AI now "finishes" anyway
    tokio::time::sleep(ms(150)).await;
    // Everything the run kept, from the first frame: it ends once, with the cancel, and nothing
    // follows that ending.
    let mut c2 = Client::dial(&url).await;
    c2.send(json!({"type": "resume", "id": "lc1", "next": 0})).await;
    let all = c2.until_quiet(ms(400)).await;
    let (n, last) = terminals(&all);
    assert!(n == 1 && last == "error:cancelled", "an answer ends exactly once, with the cancel: {n}, {last}\n{all:?}");
    let end = all.last().unwrap();
    assert!(!end["seq"].is_null() && end["type"] == "error", "no frame may follow the ending: {end}");
}

#[tokio::test]
async fn the_same_id_twice_and_a_second_question_are_answered_not_started() {
    let release = Arc::new(Notify::new());
    let started = Arc::new(Mutex::new(0));
    let (r2, s2) = (release.clone(), started.clone());
    let url = start(ai(move |_, _, s, _| {
        let (release, started) = (r2.clone(), s2.clone());
        async move {
            *started.lock().unwrap() += 1;
            text(&s, "あ\n");
            release.notified().await;
            Ok("あ".to_string())
        }
    }))
    .await;
    let mut c = Client::dial(&url).await;
    c.send(ask("d1", "", "q")).await;
    loop {
        if c.recv().await.unwrap()["type"] == "line" {
            break;
        }
    }
    c.send(ask("d2", "", "another while d1 runs")).await;
    let busy = loop {
        let m = c.recv().await.unwrap();
        if m["type"] == "error" {
            break m;
        }
    };
    assert!(busy["code"] == "busy" && busy["id"] == "d2", "a second question on a busy connection gets busy: {busy}");
    let mut c2 = Client::dial(&url).await;
    c2.send(ask("d1", "", "same id again")).await;
    let dup = loop {
        let m = c2.recv().await.unwrap();
        if m["type"] == "error" {
            break m;
        }
    };
    assert!(dup["code"] == "bad_request" && dup["message"] == "duplicate id", "a repeated id is refused: {dup}");
    assert_eq!(*started.lock().unwrap(), 1, "neither refused question may reach the AI");
    release.notify_one();
}

#[tokio::test]
async fn the_newest_connection_receives_the_answer() {
    let release = Arc::new(Notify::new());
    let r2 = release.clone();
    let url = start(ai(move |_, _, s, _| {
        let release = r2.clone();
        async move {
            text(&s, "あ\n");
            release.notified().await;
            text(&s, "い\n");
            Ok("あい".to_string())
        }
    }))
    .await;
    let mut c1 = Client::dial(&url).await;
    c1.send(ask("n1", "", "q")).await;
    loop {
        if c1.recv().await.unwrap()["type"] == "line" {
            break;
        }
    }
    let mut c2 = Client::dial(&url).await; // a second connection takes the answer over while the first is still open
    c2.send(json!({"type": "resume", "id": "n1", "next": 1})).await;
    tokio::time::sleep(ms(50)).await;
    release.notify_one();
    let mut seqs = vec![];
    loop {
        let m = c2.recv().await.unwrap();
        if m["type"] == "line" || m["type"] == "done" {
            seqs.push(m["seq"].as_i64().unwrap());
        }
        if m["type"] == "done" {
            break;
        }
    }
    assert!(seqs.len() >= 2 && seqs[0] == 1, "the new connection gets the rest from frame 1: {seqs:?}");
    // the first connection is no longer the receiver: nothing more arrives for it
    while let Some(m) = c1.recv_within(ms(200)).await {
        assert!(m["type"] != "line" && m["type"] != "done", "the replaced connection still receives frames: {m}");
    }
}

#[tokio::test]
async fn the_welcome_names_the_ai_behind_the_bridge() {
    let url = start(ai(|_, _, _, _| async { Ok(String::new()) })).await;
    let got = converse(&url, vec![hello()], |m| m["type"] == "welcome").await;
    assert_eq!(got[0]["provider"], "test AI", "the plugin shows this name in its settings: {}", got[0]);
}

#[tokio::test]
async fn a_failure_of_the_ai_ends_the_answer_with_a_provider_error_not_naming_any_ai() {
    let url = start(ai(|_, _, _, _| async { Err(AskError::Failed("the service is overloaded".into())) })).await;
    let mut c = Client::dial(&url).await;
    c.send(ask("pf1", "", "q")).await;
    let m = c.last_frame().await;
    assert!(m["code"] == "provider_failed" && m["message"] == "the service is overloaded" && !m["seq"].is_null(), "{m}");
}

#[tokio::test]
async fn forget_note_forgets_only_that_note() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded_store(dir.path(), &["/n/a.note", "/n/b.note"]);
    let st = store.clone();
    let url = start_with(ai(|_, _, _, _| async { Ok(String::new()) }), move |d| {
        d.history = st;
        d.history_chars = 1000;
    })
    .await;
    let got = converse(&url, vec![hello(), json!({"type": "forgetNote", "note": "/n/a.note"})], |m| m["type"] == "historyCleared").await;
    assert_eq!(got[1]["notes"], 1, "{got:?}");
    assert!(store.recent("/n/a.note", 1000).unwrap().is_empty(), "the named note must be forgotten");
    assert_eq!(store.recent("/n/b.note", 1000).unwrap().len(), 1, "the other note must be kept");
}

#[tokio::test]
async fn forget_note_without_a_real_key_is_refused_and_forgets_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded_store(dir.path(), &["/n/a.note"]);
    let st = store.clone();
    let url = start_with(ai(|_, _, _, _| async { Ok(String::new()) }), move |d| {
        d.history = st;
        d.history_chars = 1000;
    })
    .await;
    for note in [Some(json!("")), Some(json!("x".repeat(5000))), None] {
        let mut frame = json!({"type": "forgetNote"});
        if let Some(n) = note {
            frame["note"] = n;
        }
        let got = converse(&url, vec![hello(), frame], |m| m["type"] == "error").await;
        assert_eq!(got.last().unwrap()["code"], "bad_request", "a request without a real key is refused: {got:?}");
    }
    assert_eq!(store.recent("/n/a.note", 1000).unwrap().len(), 1, "a refused request forgets nothing");
}

#[tokio::test]
async fn clear_history_stays_simple_whatever_else_is_in_the_frame() {
    // clearHistory means "every note", whatever else is in the frame: a field added by a newer
    // plugin must never narrow it silently, and never widen anything else.
    let dir = tempfile::tempdir().unwrap();
    let store = seeded_store(dir.path(), &["/n/a.note"]);
    let st = store.clone();
    let url = start_with(ai(|_, _, _, _| async { Ok(String::new()) }), move |d| {
        d.history = st;
        d.history_chars = 1000;
    })
    .await;
    let got = converse(&url, vec![hello(), json!({"type": "clearHistory", "note": "/n/other.note"})], |m| m["type"] == "historyCleared").await;
    assert_eq!(got[1]["notes"], 1, "{got:?}");
}

#[tokio::test]
async fn health_is_answered_without_a_token() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let url = start(ai(|_, _, _, _| async { Ok(String::new()) })).await;
    let addr = url.trim_start_matches("ws://").trim_end_matches("/v1").to_string();
    let mut s = tokio::net::TcpStream::connect(&addr).await.unwrap();
    s.write_all(b"GET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").await.unwrap();
    let mut buf = String::new();
    s.read_to_string(&mut buf).await.unwrap();
    assert!(buf.contains("{\"ok\":true,\"protocol\":1}"), "{buf}");
}

#[tokio::test]
async fn an_answer_that_outgrows_its_pages_is_cut_and_the_ai_is_stopped() {
    let stopped = Arc::new(Notify::new());
    let st = stopped.clone();
    let url = start(ai(move |_, _, s, c| {
        let stopped = st.clone();
        async move {
            // far more than one page holds (10 characters a line, about 16 lines a page)
            for _ in 0..100 {
                if c.is_cancelled() {
                    stopped.notify_one();
                    return Err(AskError::Cancelled(String::new()));
                }
                text(&s, &"あ".repeat(20));
                tokio::time::sleep(ms(5)).await;
            }
            Ok(String::new())
        }
    }))
    .await;
    let mut c = Client::dial(&url).await;
    let mut a = ask("p1", "", "q");
    a["layout"]["maxPages"] = json!(1);
    c.send(a).await;
    let m = c.last_frame().await;
    assert!(m["type"] == "done" && m["truncated"] == true, "an answer that does not fit says so: {m}");
    tokio::time::timeout(Duration::from_secs(5), stopped.notified()).await.expect("the AI must stop when no more can be written");
}

/// Small frames must not wait for a delayed acknowledgement (Nagle's algorithm): the handshake and
/// the welcome of a session on the loopback take a few milliseconds, not the ~40 ms a stalled
/// small write costs. The Go bridge gets this from its HTTP server; here it is set on the listener.
#[tokio::test]
async fn small_frames_are_not_held_back_by_nagle() {
    use spellink_bridge::server::{Deps, http};
    use std::net::SocketAddr;
    let deps: Deps = default_deps(ai(|_, _, _, _| async { Ok(String::new()) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    // the same listener as the bridge's `serve` and the relay
    let listener = spellink_bridge::util::low_latency(listener);
    let app = http::router(Arc::new(deps), None, None).into_make_service_with_connect_info::<SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let url = format!("ws://{addr}/v1");
    let mut times = vec![];
    for _ in 0..7 {
        let t = std::time::Instant::now();
        let (mut ws, _) = tokio_tungstenite::connect_async_with_config(&url, None, true).await.unwrap();
        use futures_util::{SinkExt, StreamExt};
        let (init, m1) = spellink_bridge::securechan::Initiator::start(&spellink_bridge::securechan::psk("secret")).unwrap();
        ws.send(tokio_tungstenite::tungstenite::Message::Binary(m1.into())).await.unwrap();
        let _ = ws.next().await.unwrap().unwrap(); // message 2
        let _ = init;
        let _ = ws.next().await.unwrap().unwrap(); // the welcome
        times.push(t.elapsed());
    }
    times.sort();
    // the fastest of 7: a busy machine can slow some openings, but with Nagle's algorithm every
    // opening waits 40 ms or more, so none would be quick
    assert!(times[0] < Duration::from_millis(25), "the fastest of 7 sessions opened in {:?}: small frames are being held back", times[0]);
}

#[tokio::test]
async fn a_probe_is_answered_with_the_size_it_carried_and_stores_nothing() {
    let url = start(ai(|_, _, _, _| async { Ok(String::new()) })).await;
    let data = json!({"strokes": [[1, 2, 3], [4, 5, 6]]});
    let want = data.to_string().len();
    let got = converse(&url, vec![hello(), json!({"type": "probe", "id": "p1", "data": data})], |m| m["type"] == "probeAck").await;
    assert_eq!(got[1]["id"], "p1", "{got:?}");
    assert_eq!(got[1]["bytes"], want, "{got:?}");
}

#[tokio::test]
async fn a_probe_without_an_id_is_refused() {
    let url = start(ai(|_, _, _, _| async { Ok(String::new()) })).await;
    let got = converse(&url, vec![hello(), json!({"type": "probe", "data": {}})], |m| m["type"] == "error").await;
    assert_eq!(got[1]["code"], "bad_request", "{got:?}");
}

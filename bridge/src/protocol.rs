//! The wire format between the plugin and the bridge: JSON text frames over one WebSocket
//! (Go: internal/protocol). The plugin keeps its own copy in plugin/src/protocol.ts — change both
//! together. The shapes are pinned by plugin/__tests__/contract.test.ts against a recording of
//! the frames the bridge really sends (bridge/internal/server/testdata/frames.golden.json).

use crate::layout::{Layout, Line as InkLine};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const VERSION: i64 = 1;
pub const WS_PATH: &str = "/v1";

fn is_empty(s: &String) -> bool {
    s.is_empty()
}
fn is_zero(n: &i64) -> bool {
    *n == 0
}

/// Any frame the plugin sends; `type` selects which fields are set.
///   - "hello":  token, client — must be the first frame
///   - "ask":    id, question, lang ("ja" | "en"), layout, note (key of the open note; earlier
///     questions asked in the same note are handed to the model, ADR 0012)
///   - "cancel": id
///   - "resume": id, next — after a dropped connection: send the answer's frames from number
///     `next` on (every line/done/error frame of an answer carries `seq`, counting from 0; ADR 0013)
///   - "report": data — free-form diagnostics appended to the bridge's report log
///   - "setSettings": settings — change user settings stored on the bridge
///   - "clearHistory": forget the conversations of every note
///   - "forgetNote": note — forget the conversation of that note only
///   - "probe": id, data — a speed measurement: the bridge stores nothing and answers `probeAck`
///     with the size of `data` as it arrived, so the plugin can time a payload of a known size
///   - "pairRequest": id (the device's id), gen, code (eight digits shown on the device) — only
///     on the pairing channel (a session opened with B); the owner confirms the code at the bridge
///   - "pairAck": id, gen — the device has stored K (pairing channel)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClientMessage {
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "is_empty")]
    pub token: String,
    #[serde(default, skip_serializing_if = "is_empty")]
    pub client: String,
    #[serde(default, skip_serializing_if = "is_empty")]
    pub id: String,
    #[serde(default, skip_serializing_if = "is_empty")]
    pub question: String,
    #[serde(default, skip_serializing_if = "is_empty")]
    pub note: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub next: i64,
    #[serde(default, skip_serializing_if = "is_empty")]
    pub lang: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<Layout>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<Settings>,
    #[serde(default, rename = "gen", skip_serializing_if = "is_zero")]
    pub generation: i64,
    #[serde(default, skip_serializing_if = "is_empty")]
    pub code: String,
}

impl ClientMessage {
    /// Reads one frame. It must be a JSON object (serde would also accept an array for a struct;
    /// the Go bridge does not, and neither does the plugin's protocol).
    pub fn parse(bytes: &[u8]) -> Result<ClientMessage, String> {
        let v: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if !v.is_object() {
            return Err("a frame is a JSON object".into());
        }
        serde_json::from_value(v).map_err(|e| e.to_string())
    }
}

/// The user's plugin preferences. The plugin cannot persist data on the device, so the bridge
/// stores them and hands them over in every welcome.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Opens the progress panel while answering (debug view). Off: the answer is just written onto
    /// the page; the panel appears only on errors.
    #[serde(default)]
    pub show_progress: bool,
    /// Applies the agent's writing slope (`tilt_degrees`) to answers.
    #[serde(default)]
    pub line_tilt: bool,
    /// The agent's own habit: positive rises to the right. Chosen once per bridge; read-only for
    /// the plugin.
    #[serde(default)]
    pub tilt_degrees: f64,
    /// Draws a wave on the page while the AI thinks.
    #[serde(default)]
    pub thinking_animation: bool,
    /// Letter spacing in percent of the glyph's natural advance, for full-width (kanji, kana) and
    /// half-width (Latin, digits) characters. 100 is the tightest the glyph data allows.
    #[serde(default)]
    pub full_width_spacing: i64,
    #[serde(default)]
    pub half_width_spacing: i64,
    /// Reads the page before answering so the answer stays clear of existing writing (and moves to
    /// a new page when there is no room). It costs a save and a read of the whole page, so it is
    /// off unless turned on.
    #[serde(default)]
    pub avoid_overwriting: bool,
    /// Shows a short notice when the user asks again while an answer is being written.
    #[serde(default)]
    pub busy_notice: bool,
}

// Limits of the spacing sliders (percent).
pub const SPACING_MIN: i64 = 100;
pub const FULL_SPACING_MAX: i64 = 140;
pub const HALF_SPACING_MAX: i64 = 160;

/// Confirms a setSettings.
#[derive(Debug, Serialize)]
pub struct SettingsReply {
    #[serde(rename = "type")]
    pub kind: &'static str, // "settings"
    pub settings: Settings,
}

/// Confirms a clearHistory or forgetNote.
#[derive(Debug, Serialize)]
pub struct HistoryCleared {
    #[serde(rename = "type")]
    pub kind: &'static str, // "historyCleared"
    pub notes: usize,
}

#[derive(Debug, Serialize)]
pub struct Welcome {
    #[serde(rename = "type")]
    pub kind: &'static str, // "welcome"
    pub bridge: String,
    /// The AI the bridge asks ("Claude Code"), shown in the plugin's settings.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub provider: String,
    pub version: String,
    pub protocol: i64,
    pub settings: Settings,
}

#[derive(Debug, Serialize)]
pub struct Status {
    #[serde(rename = "type")]
    pub kind: &'static str, // "status"
    pub id: String,
    /// "thinking" | "writing" | "alive" (a keep-alive: the answer is still being produced)
    pub stage: &'static str,
    /// With a keep-alive: where the AI is ("starting": the process runs but the model has not
    /// begun; "working": it has begun, no text yet; "writing") and how long since it last showed a
    /// sign of life (always present on a keep-alive, even when 0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(rename = "idleMs", skip_serializing_if = "Option::is_none")]
    pub idle_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct LineFrame {
    #[serde(rename = "type")]
    pub kind: &'static str, // "line"
    pub id: String,
    pub seq: i64,
    #[serde(flatten)]
    pub line: InkLine,
}

/// The answer to a "probe": its id and the size, in bytes, of the data it carried.
#[derive(Debug, Serialize)]
pub struct ProbeAck {
    #[serde(rename = "type")]
    pub kind: &'static str, // "probeAck"
    pub id: String,
    pub bytes: usize,
}

#[derive(Debug, Serialize)]
pub struct Done {
    #[serde(rename = "type")]
    pub kind: &'static str, // "done"
    pub id: String,
    pub seq: i64,
    pub text: String,
    pub missing: Vec<String>,
    pub truncated: bool,
    pub pages: i64,
    #[serde(rename = "elapsedMs")]
    pub elapsed_ms: i64,
}

/// Error codes: unauthorized, bad_request, busy, provider_failed, cancelled, unknown_ask,
/// provider_timeout.
#[derive(Debug, Serialize)]
pub struct ErrorFrame {
    #[serde(rename = "type")]
    pub kind: &'static str, // "error"
    #[serde(skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// Set when the error ends an answer, like line and done.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,
    pub code: String,
    pub message: String,
}

impl ErrorFrame {
    pub fn new(id: &str, code: &str, message: &str) -> ErrorFrame {
        ErrorFrame { kind: "error", id: id.to_string(), seq: None, code: code.to_string(), message: message.to_string() }
    }
}

/// Gives a device its key K once the owner has confirmed the pairing (ADR 0016). It is only ever
/// sent inside the encrypted pairing channel.
#[derive(Debug, Serialize)]
pub struct PairGrant {
    #[serde(rename = "type")]
    pub kind: &'static str, // "pairGrant"
    pub key: String,
    pub endpoints: Vec<String>,
}

/// Refuses a pairing request and ends the session. Reasons: code_in_use, pair_busy, grant_pending,
/// window_closed, expired, superseded, unknown_device.
#[derive(Debug, Serialize)]
pub struct PairDenied {
    #[serde(rename = "type")]
    pub kind: &'static str, // "pairDenied"
    pub reason: String,
}

pub fn to_frame<T: Serialize>(v: &T) -> Option<Vec<u8>> {
    serde_json::to_vec(v).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_client_message_reads_what_the_plugin_sends() {
        let m =
            ClientMessage::parse(br#"{"type":"ask","id":"q1","question":"hi","lang":"en","note":"n","layout":{"x":1,"y":2,"width":3,"pageHeight":4,"bottomMargin":5,"charSize":6,"maxPages":2},"extra":true}"#).unwrap();
        assert_eq!((m.kind.as_str(), m.id.as_str(), m.lang.as_str()), ("ask", "q1", "en"));
        let l = m.layout.unwrap();
        assert_eq!((l.page_height, l.max_pages), (4.0, 2));
        assert!(ClientMessage::parse(br#"{"type":5}"#).is_err(), "a type that is not text is not a message");
        for bad in ["[]", "null", "5", "\"x\"", "{", ""] {
            assert!(ClientMessage::parse(bad.as_bytes()).is_err(), "{bad:?}");
        }
        assert_eq!(ClientMessage::parse(b"{}").unwrap().kind, "", "an empty object is a message with no type");
    }

    #[test]
    fn frames_have_the_shape_the_plugin_validates() {
        let line = LineFrame { kind: "line", id: "q".into(), seq: 3, line: InkLine { index: 0, page: 1, text: "あ".into(), strokes: vec![], cont: false } };
        assert_eq!(serde_json::to_string(&line).unwrap(), r#"{"type":"line","id":"q","seq":3,"index":0,"page":1,"text":"あ","strokes":[]}"#);
        let alive = Status { kind: "status", id: "q".into(), stage: "alive", phase: Some("working".into()), idle_ms: Some(0) };
        assert_eq!(serde_json::to_string(&alive).unwrap(), r#"{"type":"status","id":"q","stage":"alive","phase":"working","idleMs":0}"#, "a keep-alive always carries idleMs, even when 0");
        let thinking = Status { kind: "status", id: "q".into(), stage: "thinking", phase: None, idle_ms: None };
        assert_eq!(serde_json::to_string(&thinking).unwrap(), r#"{"type":"status","id":"q","stage":"thinking"}"#);
        let done = Done { kind: "done", id: "q".into(), seq: 4, text: "t".into(), missing: vec![], truncated: false, pages: 1, elapsed_ms: 12 };
        assert_eq!(serde_json::to_string(&done).unwrap(), r#"{"type":"done","id":"q","seq":4,"text":"t","missing":[],"truncated":false,"pages":1,"elapsedMs":12}"#);
        let mut e = ErrorFrame::new("", "bad_request", "m");
        assert_eq!(serde_json::to_string(&e).unwrap(), r#"{"type":"error","code":"bad_request","message":"m"}"#);
        e.id = "q".into();
        e.seq = Some(0);
        assert_eq!(serde_json::to_string(&e).unwrap(), r#"{"type":"error","id":"q","seq":0,"code":"bad_request","message":"m"}"#, "seq 0 is still a seq");
    }

    #[test]
    fn settings_use_the_plugins_names() {
        let s = Settings { show_progress: true, line_tilt: true, tilt_degrees: 0.4, thinking_animation: false, full_width_spacing: 100, half_width_spacing: 110, avoid_overwriting: false, busy_notice: true };
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            r#"{"showProgress":true,"lineTilt":true,"tiltDegrees":0.4,"thinkingAnimation":false,"fullWidthSpacing":100,"halfWidthSpacing":110,"avoidOverwriting":false,"busyNotice":true}"#
        );
    }
}

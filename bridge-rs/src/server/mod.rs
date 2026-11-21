//! The bridge's HTTP/WebSocket front: one WebSocket per plugin session, authenticated by a shared
//! token or a device's secret, answering one question at a time (Go: internal/server).

pub mod admin;
pub mod http;
pub mod run;
pub mod run_table;
pub mod session;
pub mod transport;

use crate::devices::Registry;
use crate::glyphs::Font;
use crate::history;
use crate::protocol::Settings;
use crate::provider::Provider;
use run::Runs;
use serde_json::Value;
use session::Sessions;
use std::sync::Arc;
use std::time::Duration;

/// A sink for log lines.
pub type LogFn = Arc<dyn Fn(&str) + Send + Sync>;

pub struct Deps {
    pub name: String,
    pub version: String,
    pub token: String,
    pub font: Option<Arc<Font>>,
    /// The AI behind the bridge (ADR 0015); the server does not know which one.
    pub provider: Arc<dyn Provider>,
    pub report: Arc<dyn Fn(&str, Option<&Value>) + Send + Sync>,
    pub log: LogFn,
    /// The stored user settings; `set_settings` changes and persists them.
    pub settings: Arc<dyn Fn() -> Settings + Send + Sync>,
    pub set_settings: Arc<dyn Fn(Settings) -> Result<(), String> + Send + Sync>,
    /// Remembers each note's conversation (`history_chars` 0: every question stands alone).
    pub history: Arc<history::Store>,
    pub history_chars: i64,
    /// The answers in flight; share one between the direct and the relay entrance so a plugin can
    /// resume on either.
    pub runs: Arc<Runs>,
    /// Bound the AI's silence (zero: defaults 10 s / 10 s / 30 s).
    pub first_output: Duration,
    pub gap: Duration,
    pub total: Duration,
    /// How often a running answer tells the plugin it is alive (zero: 5 s).
    pub keepalive_every: Duration,
    /// How long an answer is still produced with no plugin connected (zero: default).
    pub orphan_grace: Duration,
    /// The ledger of paired devices (ADR 0016). Without it only the shared token works.
    pub devices: Option<Registry>,
    /// The live device sessions, so a revoked device can be cut off; share one between the direct
    /// and the relay entrance.
    pub sessions: Arc<Sessions>,
    /// Bound a pairing channel (zero: defaults 10 s / 5 min).
    pub pair_request_wait: Duration,
    pub pairing_max: Duration,
}

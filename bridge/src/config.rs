//! The bridge settings and diagnostic reports in the user's config directory
//! (~/.config/spellink-bridge on Linux, ~/Library/Application Support/spellink-bridge on macOS;
//! Go: internal/config). The directory is a parameter (`Store`), so each test has its own.

use crate::lan;
use crate::util;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn is_empty_str(s: &String) -> bool {
    s.is_empty()
}
fn is_zero(n: &i64) -> bool {
    *n == 0
}
fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    /// Name shown to the user in the plugin; defaults to the host name.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub port: i64,
    /// The shared secret the plugin must present. Unrelated to any Anthropic credential.
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub claude_executable: String,
    /// Passed to `claude --model` (e.g. "haiku"); empty uses Claude Code's default.
    #[serde(default, skip_serializing_if = "is_empty_str")]
    pub model: String,
    /// Chooses the AI the bridge asks (ADR 0015). Without it: Claude Code, with the
    /// claudeExecutable and model above (kept so existing config files keep working).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderConfig>,
    /// Additional addresses the plugin may try, such as a tunnel that reaches this bridge from
    /// outside the LAN (e.g. "wss://bridge.example.com/v1").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_urls: Vec<String>,
    /// A Spellink relay (e.g. "wss://relay.example.com") the bridge registers with so the plugin
    /// can reach it from outside the LAN (ADR 0010). Empty disables the relay.
    #[serde(default, skip_serializing_if = "is_empty_str")]
    pub relay_url: String,
    /// More relays, for several regions (e.g. "wss://jp.relay…" and "wss://us.relay…"). The bridge
    /// registers with every relay it is given (this and `relayUrl` together); a device tries all of
    /// them at once, so the nearest answers first and one that is down does not matter.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relay_urls: Vec<String>,
    /// The plugin's debug view setting, changed from the plugin.
    #[serde(default, skip_serializing_if = "is_false")]
    pub show_progress: bool,
    /// Defaults to on; None means "not set".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_tilt: Option<bool>,
    /// This agent's handwriting slope (positive rises to the right), picked at random the first
    /// time so every agent has its own habit. Edit to taste.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tilt_degrees: Option<f64>,
    /// Defaults to off (it saves and re-reads the note, which can stall the note app on large notes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_animation: Option<bool>,
    /// Defaults to off (it saves and reads the whole page for every question).
    #[serde(default, skip_serializing_if = "is_false")]
    pub avoid_overwriting: bool,
    /// Defaults to on; None means "not set".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub busy_notice: Option<bool>,
    /// Letter spacing in percent; None means the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_width_spacing: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub half_width_spacing: Option<i64>,
    /// How long the AI may stay silent (bridge-run spec, R13). Zero fields use the defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeouts: Option<Timeouts>,
    /// What the bridge remembers of each note's conversation (ADR 0012).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<History>,
    /// Direct wss:// at home with the bridge's own Let's Encrypt certificate (ADR 0017). Off unless `enabled`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lan: Option<lan::Config>,
    /// Tunes how devices are added (ADR 0016). Zero fields use the defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pairing: Option<Pairing>,
    /// What the AI is told about its surroundings and which tools it may use (ADR 0022).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai: Option<Ai>,
    /// The keys of config.json this version does not know (written by a newer version, or by
    /// hand), kept so that saving never deletes them.
    #[serde(flatten)]
    pub unknown: Map<String, Value>,
}

/// `ai` in config.json (ADR 0022, docs/spec/ai-tools.md). An unknown key is refused, so a
/// misspelt `deny` or `allow` cannot silently drop the owner's intent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Ai {
    /// Tell the AI the date, time zone and kind of OS. On unless `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<AiTools>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiTools {
    /// Claude Code permission rules, only `Bash(<command pattern>)`. Tools are on only when this
    /// is not empty.
    #[serde(default)]
    pub allow: Vec<String>,
}

/// The most rules, and the longest rule (characters), `ai.tools.allow` takes.
const MAX_TOOL_RULES: usize = 32;
const MAX_TOOL_RULE_CHARS: usize = 256;

/// Checks one rule of `ai.tools.allow` (cases S3-S5): the form `Bash(<pattern>)` with a pattern
/// that is more than a wildcard. "Allow everything" is deliberately not expressible.
fn check_tool_rule(rule: &str) -> Result<(), String> {
    if rule.is_empty() || rule.trim() != rule || rule.chars().any(char::is_control) || rule.chars().count() > MAX_TOOL_RULE_CHARS {
        return Err(format!("must be 1-{MAX_TOOL_RULE_CHARS} characters with no leading or trailing space and no control characters"));
    }
    let Some(inner) = rule.strip_prefix("Bash(").and_then(|r| r.strip_suffix(')')) else {
        return Err("must have the form Bash(<command>): only Bash is supported in this version".into());
    };
    if inner.trim().chars().all(|c| c == '*' || c == ':') {
        return Err("must name a command: \"allow everything\" is not supported".into());
    }
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pairing {
    /// How long the owner has to confirm a code once a device shows it (default 90).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub window_sec: i64,
    /// How long a device's first secret stays valid before it is paired (default 24).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub ttl_hours: i64,
    /// The public Spellink.snplg that `device package` adds a device's setup to.
    #[serde(default, skip_serializing_if = "is_empty_str")]
    pub plugin_package: String,
}

const DEFAULT_WINDOW_SEC: i64 = 90;
const DEFAULT_TTL_HOURS: i64 = 24;
const MIN_WINDOW_SEC: i64 = 10;
const MAX_WINDOW_SEC: i64 = 600;
const MAX_TTL_HOURS: i64 = 24 * 30;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    /// "claude-code" (the only one so far).
    #[serde(default, skip_serializing_if = "is_empty_str")]
    pub kind: String,
    /// Passed to the AI (e.g. "haiku"); empty uses its default.
    #[serde(default, skip_serializing_if = "is_empty_str")]
    pub model: String,
    /// The CLI to run, for a CLI provider; empty means its usual name.
    #[serde(default, skip_serializing_if = "is_empty_str")]
    pub executable: String,
}

/// The values `provider.kind` may have.
pub const KNOWN_PROVIDERS: &[&str] = &["claude-code"];

/// Seconds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Timeouts {
    /// How long the model may take to show its first sign of work after the process started
    /// (default 10; measured: about 1.5 s).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub first_output_sec: i64,
    /// The longest silence once it works (default 10): nothing at all, thinking included.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub gap_sec: i64,
    /// Bounds the whole answer (default 30). The silence limits already stop an AI that has
    /// stopped; this only ends one that keeps trickling.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub total_sec: i64,
}

const DEFAULT_FIRST_OUTPUT_SEC: i64 = 10;
const DEFAULT_GAP_SEC: i64 = 10;
const DEFAULT_TOTAL_SEC: i64 = 30;
const MAX_TIMEOUT_SEC: i64 = 3600;

/// How much of a note's earlier conversation is handed to the model with a new question.
/// Everything is kept on disk regardless; these only bound what is sent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct History {
    /// The size (characters of questions plus answers) of the most recent conversation sent along;
    /// 0 means the default, negative turns the memory off.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub context_chars: i64,
    /// What happens to conversation older than that. Only "none" (it is simply not sent) exists so
    /// far; "summarize" is reserved for when long notes need it.
    #[serde(default, skip_serializing_if = "is_empty_str")]
    pub compaction: String,
}

const DEFAULT_HISTORY_CHARS: i64 = 8000;
const DEFAULT_FULL_SPACING: i64 = 100;
const DEFAULT_HALF_SPACING: i64 = 110;
/// When reports.jsonl is moved aside to reports.jsonl.1 (one old copy is kept).
const MAX_REPORT_BYTES: u64 = 5 << 20;

/// A relay address is only a scheme (`wss`, or `ws` for a relay on a trusted machine), a host and a
/// port: the bridge appends the paths itself. Anything else is not used.
pub fn relay_base(s: &str) -> Option<String> {
    let u = url::Url::parse(s.trim()).ok()?;
    let ok = matches!(u.scheme(), "wss" | "ws") && u.host_str().is_some() && u.username().is_empty() && u.password().is_none() && u.query().is_none() && u.fragment().is_none() && (u.path() == "" || u.path() == "/");
    ok.then(|| u.as_str().trim_end_matches('/').to_string())
}

impl Config {
    /// Every relay to register with: `relayUrl`, then `relayUrls`, each only once. Entries that are
    /// not a relay address are left out (see [`Config::bad_relays`]).
    pub fn relays(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for r in std::iter::once(&self.relay_url).chain(self.relay_urls.iter()).filter(|r| !r.trim().is_empty()) {
            if let Some(b) = relay_base(r) {
                if !out.contains(&b) {
                    out.push(b);
                }
            }
        }
        out
    }

    /// The entries of `relayUrl` / `relayUrls` that are not usable, for the log.
    pub fn bad_relays(&self) -> Vec<String> {
        std::iter::once(&self.relay_url).chain(self.relay_urls.iter()).filter(|r| !r.trim().is_empty() && relay_base(r).is_none()).cloned().collect()
    }

    /// How long the owner has to confirm a code.
    pub fn pairing_window(&self) -> Duration {
        let s = self.pairing.as_ref().map_or(0, |p| p.window_sec);
        Duration::from_secs((if s == 0 { DEFAULT_WINDOW_SEC } else { s }) as u64)
    }

    /// How long a device's first secret stays valid.
    pub fn pairing_ttl(&self) -> Duration {
        let h = self.pairing.as_ref().map_or(0, |p| p.ttl_hours);
        Duration::from_secs((if h == 0 { DEFAULT_TTL_HOURS } else { h }) as u64 * 3600)
    }

    /// The configured base package, or the empty string.
    pub fn plugin_package(&self) -> &str {
        self.pairing.as_ref().map_or("", |p| p.plugin_package.as_str())
    }

    /// The effective kind, model and executable: provider.* wins over the older top-level
    /// claudeExecutable / model.
    pub fn provider_settings(&self) -> (String, String, String) {
        let (mut kind, mut model, mut exe) = ("claude-code".to_string(), self.model.clone(), self.claude_executable.clone());
        if let Some(p) = &self.provider {
            if !p.kind.is_empty() {
                kind = p.kind.clone();
            }
            if !p.model.is_empty() {
                model = p.model.clone();
            }
            if !p.executable.is_empty() {
                exe = p.executable.clone();
            }
        }
        if exe.is_empty() {
            exe = "claude".into();
        }
        (kind, model, exe)
    }

    /// The effective silence limits (first output, gap, total).
    pub fn provider_timeouts(&self) -> (Duration, Duration, Duration) {
        let t = self.timeouts.clone().unwrap_or_default();
        let pick = |v: i64, def: i64| Duration::from_secs((if v == 0 { def } else { v }) as u64);
        (pick(t.first_output_sec, DEFAULT_FIRST_OUTPUT_SEC), pick(t.gap_sec, DEFAULT_GAP_SEC), pick(t.total_sec, DEFAULT_TOTAL_SEC))
    }

    /// Whether the AI is told the date, time zone and kind of OS: on unless turned off.
    pub fn ai_environment(&self) -> bool {
        self.ai.as_ref().and_then(|a| a.environment) != Some(false)
    }

    /// The tool rules to hand to the AI, each once (case S6), in the order written. Empty means
    /// tools stay off.
    pub fn tool_rules(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for r in self.ai.iter().filter_map(|a| a.tools.as_ref()).flat_map(|t| t.allow.iter()) {
            if !out.contains(r) {
                out.push(r.clone());
            }
        }
        out
    }

    /// How many rules of `ai.tools.allow` repeat an earlier one, for the log.
    pub fn duplicate_tool_rules(&self) -> usize {
        self.ai.iter().filter_map(|a| a.tools.as_ref()).map(|t| t.allow.len()).sum::<usize>() - self.tool_rules().len()
    }

    /// The size of conversation to send, or 0 when the memory is off.
    pub fn history_chars(&self) -> i64 {
        match self.history.as_ref().map_or(0, |h| h.context_chars) {
            0 => DEFAULT_HISTORY_CHARS,
            n if n < 0 => 0,
            n => n,
        }
    }

    pub fn validate(&self, file: &str) -> Result<(), String> {
        let (kind, _, _) = self.provider_settings();
        if !KNOWN_PROVIDERS.contains(&kind.as_str()) {
            return Err(format!("{file}: provider.kind {kind:?} is not one this version knows ({})", KNOWN_PROVIDERS.join(", ")));
        }
        if !(0..=65535).contains(&self.port) {
            return Err(format!("{file}: port {} is not a valid port (1-65535)", self.port));
        }
        if let Some(t) = &self.timeouts {
            for (name, v) in [("firstOutputSec", t.first_output_sec), ("gapSec", t.gap_sec), ("totalSec", t.total_sec)] {
                if !(0..=MAX_TIMEOUT_SEC).contains(&v) {
                    return Err(format!("{file}: timeouts.{name} {v} must be between 0 (default) and {MAX_TIMEOUT_SEC} seconds"));
                }
            }
        }
        if let Some(p) = &self.pairing {
            if p.window_sec != 0 && !(MIN_WINDOW_SEC..=MAX_WINDOW_SEC).contains(&p.window_sec) {
                return Err(format!("{file}: pairing.windowSec {} must be 0 (default) or between {MIN_WINDOW_SEC} and {MAX_WINDOW_SEC} seconds", p.window_sec));
            }
            if !(0..=MAX_TTL_HOURS).contains(&p.ttl_hours) {
                return Err(format!("{file}: pairing.ttlHours {} must be between 0 (default) and {MAX_TTL_HOURS}", p.ttl_hours));
            }
        }
        if let Some(t) = self.ai.as_ref().and_then(|a| a.tools.as_ref()) {
            if t.allow.len() > MAX_TOOL_RULES {
                return Err(format!("{file}: ai.tools.allow has {} rules; at most {MAX_TOOL_RULES}", t.allow.len()));
            }
            if cfg!(windows) && !t.allow.is_empty() {
                return Err(format!("{file}: ai.tools.allow is not supported on Windows in this version (Linux and macOS only)"));
            }
            for (i, r) in t.allow.iter().enumerate() {
                check_tool_rule(r).map_err(|e| format!("{file}: ai.tools.allow[{i}] {r:?} {e}"))?;
            }
        }
        if let Some(h) = &self.history {
            if !h.compaction.is_empty() && h.compaction != "none" {
                return Err(format!("{file}: history.compaction {:?} is not supported yet (only \"none\")", h.compaction));
            }
        }
        Ok(())
    }

    /// Gives a new agent its writing habit: mostly gentle, sometimes more pronounced. Returns
    /// whether it chose one.
    pub fn ensure_tilt(&mut self) -> bool {
        if self.tilt_degrees.is_some() {
            return false;
        }
        // a normal sample (Box–Muller), kept within ±2.5 degrees and rounded to a tenth
        let (u1, u2): (f64, f64) = (1.0 - rand::random::<f64>(), rand::random::<f64>());
        let n = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
        self.tilt_degrees = Some((n.clamp(-2.5, 2.5) * 10.0).round() / 10.0);
        true
    }

    /// The animation setting: off unless turned on.
    pub fn thinking_animation_on(&self) -> bool {
        self.thinking_animation == Some(true)
    }
    pub fn full_spacing(&self) -> i64 {
        self.full_width_spacing.unwrap_or(DEFAULT_FULL_SPACING)
    }
    pub fn half_spacing(&self) -> i64 {
        self.half_width_spacing.unwrap_or(DEFAULT_HALF_SPACING)
    }
    /// The busy-notice setting: on unless turned off.
    pub fn busy_notice_on(&self) -> bool {
        self.busy_notice != Some(false)
    }
    /// The line-tilt setting: on unless turned off.
    pub fn line_tilt_on(&self) -> bool {
        self.line_tilt != Some(false)
    }
}

/// A new shared token: 24 random bytes, base64url.
fn fresh_token() -> String {
    let mut secret = [0u8; 24];
    rand::rng().fill_bytes(&mut secret);
    URL_SAFE_NO_PAD.encode(secret)
}

/// The directory where config.json, glyphs.bin, reports.jsonl and the rest live.
#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
}

/// The user's config directory (Go's `os.UserConfigDir`): $XDG_CONFIG_HOME or ~/.config on Linux,
/// ~/Library/Application Support on macOS, %AppData% on Windows.
pub fn user_config_dir() -> Result<PathBuf, String> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(windows) {
        return env("AppData").ok_or_else(|| "%AppData% is not defined".to_string());
    }
    if cfg!(target_os = "macos") {
        return env("HOME").map(|h| h.join("Library/Application Support")).ok_or_else(|| "$HOME is not defined".to_string());
    }
    match env("XDG_CONFIG_HOME") {
        Some(x) if x.is_absolute() => Ok(x),
        _ => env("HOME").map(|h| h.join(".config")).ok_or_else(|| "neither $XDG_CONFIG_HOME nor $HOME are defined".to_string()),
    }
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Store {
        Store { dir: dir.into() }
    }

    /// The store of the real user: `<config dir>/spellink-bridge`.
    pub fn user() -> Result<Store, String> {
        Ok(Store::new(user_config_dir()?.join("spellink-bridge")))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
    pub fn history_dir(&self) -> PathBuf {
        self.dir.join("history")
    }
    pub fn glyph_file(&self) -> PathBuf {
        self.dir.join("glyphs.bin")
    }
    pub fn report_file(&self) -> PathBuf {
        self.dir.join("reports.jsonl")
    }
    pub fn config_file(&self) -> PathBuf {
        self.dir.join("config.json")
    }
    /// The ledger of paired devices.
    pub fn devices_file(&self) -> PathBuf {
        self.dir.join("devices.json")
    }
    /// Holds the bearer token the command line uses to talk to the running bridge.
    pub fn admin_token_file(&self) -> PathBuf {
        self.dir.join("admin.token")
    }

    /// The command line's token, creating it (mode 0600, 32 random bytes) the first time.
    pub fn admin_token(&self) -> Result<String, String> {
        let file = self.admin_token_file();
        match std::fs::read_to_string(&file) {
            Ok(s) => {
                let tok = s.trim();
                if tok.len() >= 32 {
                    return Ok(tok.to_string());
                }
                // a token that is too short was not written by us: replace it rather than trust it
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", file.display())),
        }
        let mut raw = [0u8; 32];
        rand::rng().fill_bytes(&mut raw);
        let tok = URL_SAFE_NO_PAD.encode(raw);
        util::create_private_dir(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        util::write_private(&file, format!("{tok}\n").as_bytes()).map_err(|e| format!("{}: {e}", file.display()))?;
        Ok(tok)
    }

    /// Writes the config back (used when the plugin changes a setting). Keys this version does
    /// not know are kept (they are in `unknown`); the write is atomic.
    pub fn save(&self, c: &Config) -> Result<(), String> {
        let file = self.config_file();
        let mut out = serde_json::to_vec_pretty(c).map_err(|e| e.to_string())?;
        out.push(b'\n');
        util::write_private_atomic(&file, &out).map_err(|e| format!("{}: {e}", file.display()))
    }

    /// Reads the config, creating it with a fresh random token on first run.
    pub fn load(&self) -> Result<Config, String> {
        let file = self.config_file();
        let shown = file.display().to_string();
        match std::fs::read(&file) {
            Ok(b) => {
                let mut c: Config = serde_json::from_slice(&b).map_err(|e| format!("{shown}: {e}"))?;
                c.validate(&shown)?;
                if c.port == 0 {
                    c.port = 8765;
                }
                if c.claude_executable.is_empty() {
                    c.claude_executable = "claude".into();
                }
                // A missing or empty token is replaced, never accepted: the documented way to change
                // the token is to delete it and restart, and an empty one would let anyone in.
                let new_token = c.token.is_empty();
                if new_token {
                    c.token = fresh_token();
                }
                if c.ensure_tilt() || new_token {
                    self.save(&c)?;
                }
                Ok(c)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut c = Config { name: hostname(), port: 8765, token: fresh_token(), claude_executable: "claude".into(), ..Config::default() };
                c.ensure_tilt();
                util::create_private_dir(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
                let mut out = serde_json::to_vec_pretty(&c).map_err(|e| e.to_string())?;
                out.push(b'\n');
                util::write_private(&file, &out).map_err(|e| format!("{shown}: {e}"))?;
                Ok(c)
            }
            Err(e) => Err(format!("{shown}: {e}")),
        }
    }

    /// Adds one diagnostics entry (from the plugin) to reports.jsonl.
    pub fn append_report(&self, peer: &str, data: &Value) -> Result<(), String> {
        let file = self.report_file();
        util::create_private_dir(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        // The log is written by whoever holds the token: keep it from growing without bound.
        if std::fs::metadata(&file).is_ok_and(|m| m.len() > MAX_REPORT_BYTES) {
            let mut old = file.clone().into_os_string();
            old.push(".1");
            std::fs::rename(&file, PathBuf::from(old)).map_err(|e| format!("{}: {e}", file.display()))?;
        }
        let mut opts = std::fs::OpenOptions::new();
        opts.create(true).append(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        let mut f = opts.open(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        let mut line = serde_json::to_vec(&serde_json::json!({"at": util::now_rfc3339(), "peer": peer, "data": data})).map_err(|e| e.to_string())?;
        line.push(b'\n');
        f.write_all(&line).map_err(|e| format!("{}: {e}", file.display()))
    }
}

pub fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok().filter(|s| !s.is_empty()))
        .or_else(|| std::env::var("COMPUTERNAME").ok().filter(|s| !s.is_empty()))
        .or_else(|| std::process::Command::new("hostname").output().ok().and_then(|o| String::from_utf8(o.stdout).ok()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn store() -> (tempfile::TempDir, Store) {
        let d = tempfile::tempdir().unwrap();
        let s = Store::new(d.path().join("spellink-bridge"));
        (d, s)
    }

    fn write_config(s: &Store, body: &str) {
        std::fs::create_dir_all(s.dir()).unwrap();
        std::fs::write(s.config_file(), body).unwrap();
    }

    #[cfg(unix)]
    fn mode(p: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn load_creates_a_config_with_a_token_and_its_own_tilt() {
        let (_d, s) = store();
        let c = s.load().unwrap();
        assert!(c.token.len() >= 30 && c.port == 8765 && c.claude_executable == "claude" && c.tilt_degrees.is_some(), "{c:?}");
        #[cfg(unix)]
        assert_eq!(mode(&s.config_file()), 0o600);
        let again = s.load().unwrap();
        assert_eq!((again.token, again.tilt_degrees), (c.token, c.tilt_degrees), "the token and the slope are stable across runs");
    }

    #[test]
    fn a_relay_is_only_a_scheme_a_host_and_a_port() {
        for ok in ["wss://relay.example", "wss://relay.example/", "wss://relay.example:8443", " wss://us.relay.example ", "ws://127.0.0.1:8780"] {
            assert!(relay_base(ok).is_some(), "{ok}");
        }
        assert_eq!(relay_base("wss://relay.example/").as_deref(), Some("wss://relay.example"));
        for bad in ["", "relay.example", "https://relay.example", "wss://", "wss://relay.example/v1", "wss://relay.example?x=1", "wss://u:p@relay.example", "wss://relay.example#f"] {
            assert!(relay_base(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn relays_come_from_relay_url_then_relay_urls_without_repeats_and_json_reads_both() {
        let c: Config = serde_json::from_str(r#"{"relayUrl":"wss://jp.r.example","relayUrls":["wss://us.r.example/","wss://jp.r.example","nonsense"]}"#).unwrap();
        assert_eq!(c.relays(), vec!["wss://jp.r.example".to_string(), "wss://us.r.example".to_string()]);
        assert_eq!(c.bad_relays(), vec!["nonsense".to_string()]);
        assert!(Config::default().relays().is_empty());
        // a config written back keeps the list, and without a list writes none
        assert!(serde_json::to_string(&c).unwrap().contains("relayUrls"));
        assert!(!serde_json::to_string(&Config::default()).unwrap().contains("relayUrls"));
    }

    #[test]
    fn a_deleted_or_empty_token_is_replaced_and_saved_not_accepted() {
        let (_d, s) = store();
        for body in [r#"{"port":8765}"#, r#"{"port":8765,"token":""}"#] {
            write_config(&s, body);
            let c = s.load().unwrap();
            assert!(c.token.len() >= 30, "{body}: {c:?}");
            let again = s.load().unwrap();
            assert_eq!(again.token, c.token, "{body}: the new token is saved, not made anew at every start");
        }
    }

    #[test]
    fn load_names_the_file_when_json_is_broken() {
        let (_d, s) = store();
        write_config(&s, r#"{"name": "x"  "port": 1}"#);
        let e = s.load().unwrap_err();
        assert!(e.contains(&s.config_file().display().to_string()), "{e}");
    }

    #[test]
    fn save_round_trip_and_line_tilt_default() {
        let (_d, s) = store();
        let mut c = s.load().unwrap();
        assert!(c.line_tilt_on(), "line tilt defaults to on");
        c.line_tilt = Some(false);
        c.show_progress = true;
        s.save(&c).unwrap();
        let got = s.load().unwrap();
        assert!(!got.line_tilt_on() && got.show_progress);
    }

    #[test]
    fn the_tilt_stays_within_a_gentle_range() {
        for _ in 0..2000 {
            let mut c = Config::default();
            c.ensure_tilt();
            let d = c.tilt_degrees.unwrap();
            assert!((-2.5..=2.5).contains(&d), "tilt {d} out of range");
        }
    }

    #[test]
    fn append_report_writes_json_lines() {
        let (_d, s) = store();
        s.append_report("peer", &json!({"a": 1})).unwrap();
        s.append_report("peer", &json!({"a": 2})).unwrap();
        let text = std::fs::read_to_string(s.report_file()).unwrap();
        assert_eq!(text.lines().count(), 2);
        let first: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!((first["peer"].clone(), first["data"]["a"].clone()), (json!("peer"), json!(1)));
        assert!(first["at"].as_str().unwrap().ends_with('Z'));
        #[cfg(unix)]
        assert_eq!(mode(&s.report_file()), 0o600);
    }

    #[test]
    fn history_defaults_and_switches() {
        let chars = |h: Option<History>| Config { history: h, ..Config::default() }.history_chars();
        assert_eq!(chars(None), 8000);
        assert_eq!(chars(Some(History::default())), 8000);
        assert_eq!(chars(Some(History { context_chars: 500, ..Default::default() })), 500);
        assert_eq!(chars(Some(History { context_chars: -1, ..Default::default() })), 0);
    }

    #[test]
    fn an_unsupported_compaction_is_rejected_by_name() {
        let (_d, s) = store();
        for (mode, ok) in [("none", true), ("", true), ("summarize", false)] {
            write_config(&s, &format!(r#"{{"token":"x","history":{{"compaction":"{mode}"}}}}"#));
            let r = s.load();
            assert_eq!(r.is_ok(), ok, "compaction {mode:?}: {r:?}");
            if let Err(e) = r {
                assert!(e.contains("config.json") && e.contains("summarize"), "the error must name the file: {e}");
            }
        }
    }

    #[test]
    fn provider_timeout_defaults_and_overrides() {
        let sec = Duration::from_secs;
        assert_eq!(Config::default().provider_timeouts(), (sec(10), sec(10), sec(30)));
        let c = Config { timeouts: Some(Timeouts { first_output_sec: 5, total_sec: 60, ..Default::default() }), ..Default::default() };
        assert_eq!(c.provider_timeouts(), (sec(5), sec(10), sec(60)));
    }

    #[test]
    fn out_of_range_settings_are_rejected_by_name() {
        let (_d, s) = store();
        for (body, want) in
            [(r#"{"token":"x","port":70000}"#, "port"), (r#"{"token":"x","port":-1}"#, "port"), (r#"{"token":"x","timeouts":{"gapSec":-5}}"#, "gapSec"), (r#"{"token":"x","timeouts":{"totalSec":99999}}"#, "totalSec")]
        {
            write_config(&s, body);
            let e = s.load().unwrap_err();
            assert!(e.contains(want) && e.contains("config.json"), "{body}: want an error naming {want} and the file, got {e}");
        }
    }

    #[test]
    fn the_report_log_is_moved_aside_when_it_grows_too_large() {
        let (_d, s) = store();
        std::fs::create_dir_all(s.dir()).unwrap();
        std::fs::write(s.report_file(), vec![0u8; MAX_REPORT_BYTES as usize + 1]).unwrap();
        s.append_report("peer", &json!({"k": "v"})).unwrap();
        let old = PathBuf::from(format!("{}.1", s.report_file().display()));
        assert!(std::fs::metadata(&old).unwrap().len() > MAX_REPORT_BYTES, "the old log must be kept as .1");
        assert!(std::fs::metadata(s.report_file()).unwrap().len() < 1000, "a fresh log must start");
    }

    #[test]
    fn the_busy_notice_is_on_unless_turned_off() {
        assert!(Config::default().busy_notice_on());
        assert!(!Config { busy_notice: Some(false), ..Default::default() }.busy_notice_on());
        assert!(Config { busy_notice: Some(true), ..Default::default() }.busy_notice_on());
    }

    #[test]
    fn the_provider_defaults_to_claude_code_and_the_older_fields_still_work() {
        let t = |m: &str, e: &str| (m.to_string(), e.to_string());
        let (kind, model, exe) = Config::default().provider_settings();
        assert_eq!((kind.as_str(), (model, exe)), ("claude-code", t("", "claude")));
        let c = Config { model: "haiku".into(), claude_executable: "/opt/claude".into(), ..Default::default() };
        let (kind, model, exe) = c.provider_settings();
        assert_eq!((kind.as_str(), (model, exe)), ("claude-code", t("haiku", "/opt/claude")), "the older top-level fields count");
        let c = Config { model: "haiku".into(), provider: Some(ProviderConfig { model: "sonnet".into(), executable: "/x/claude".into(), ..Default::default() }), ..Default::default() };
        let (kind, model, exe) = c.provider_settings();
        assert_eq!((kind.as_str(), (model, exe)), ("claude-code", t("sonnet", "/x/claude")), "provider.* wins");
    }

    #[test]
    fn an_unknown_provider_is_refused_by_name_and_with_the_choices() {
        let (_d, s) = store();
        for (body, ok) in [(r#"{"token":"x"}"#, true), (r#"{"token":"x","provider":{"kind":"claude-code"}}"#, true), (r#"{"token":"x","provider":{"kind":"gpt-9"}}"#, false)] {
            write_config(&s, body);
            match (s.load(), ok) {
                (Ok(_), true) => {}
                (Err(e), false) => assert!(e.contains("gpt-9") && e.contains("claude-code") && e.contains("config.json"), "{e}"),
                (r, _) => panic!("{body}: {r:?}"),
            }
        }
    }

    #[test]
    fn pairing_defaults_and_overrides() {
        let c = Config::default();
        assert_eq!((c.pairing_window(), c.pairing_ttl(), c.plugin_package()), (Duration::from_secs(90), Duration::from_secs(24 * 3600), ""));
        let c = Config { pairing: Some(Pairing::default()), ..Default::default() };
        assert_eq!((c.pairing_window(), c.pairing_ttl()), (Duration::from_secs(90), Duration::from_secs(24 * 3600)), "zero fields mean the defaults");
        let c = Config { pairing: Some(Pairing { window_sec: 30, ttl_hours: 2, plugin_package: "/x/Spellink.snplg".into() }), ..Default::default() };
        assert_eq!((c.pairing_window(), c.pairing_ttl(), c.plugin_package()), (Duration::from_secs(30), Duration::from_secs(7200), "/x/Spellink.snplg"));
    }

    #[test]
    fn out_of_range_pairing_settings_are_rejected_by_name() {
        for (name, p) in [
            ("pairing.windowSec", Pairing { window_sec: 9, ..Default::default() }),
            ("pairing.windowSec", Pairing { window_sec: 601, ..Default::default() }),
            ("pairing.ttlHours", Pairing { ttl_hours: -1, ..Default::default() }),
            ("pairing.ttlHours", Pairing { ttl_hours: 24 * 30 + 1, ..Default::default() }),
        ] {
            let c = Config { pairing: Some(p.clone()), ..Default::default() };
            let e = c.validate("config.json").unwrap_err();
            assert!(e.contains(name), "{name} {p:?}: {e}");
        }
        let ok = Config { pairing: Some(Pairing { window_sec: 10, ttl_hours: 24 * 30, ..Default::default() }), ..Default::default() };
        assert!(ok.validate("config.json").is_ok(), "the limits themselves are allowed");
    }

    #[test]
    fn the_admin_token_is_created_once_and_private() {
        let (_d, s) = store();
        let a = s.admin_token().unwrap();
        assert!(a.len() >= 32);
        assert_eq!(a, s.admin_token().unwrap(), "the token must stay the same once created");
        #[cfg(unix)]
        assert_eq!(mode(&s.admin_token_file()), 0o600);
    }

    #[test]
    fn a_short_or_broken_admin_token_is_replaced_not_trusted() {
        let (_d, s) = store();
        std::fs::create_dir_all(s.dir()).unwrap();
        std::fs::write(s.admin_token_file(), "short\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(s.admin_token_file(), std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        let tok = s.admin_token().unwrap();
        assert!(tok.len() >= 32 && tok != "short");
        #[cfg(unix)]
        assert_eq!(mode(&s.admin_token_file()), 0o600, "a replaced token file must become private");
        // a file that cannot be read at all is an error, not a reason to create a new token
        std::fs::remove_file(s.admin_token_file()).unwrap();
        std::fs::create_dir(s.admin_token_file()).unwrap();
        assert!(s.admin_token().is_err(), "an unreadable token file must fail");
    }

    #[test]
    fn the_devices_file_lives_in_the_config_directory() {
        let (_d, s) = store();
        let f = s.devices_file();
        assert_eq!((f.parent().unwrap(), f.file_name().unwrap()), (s.dir(), std::ffi::OsStr::new("devices.json")));
    }

    #[test]
    fn saving_keeps_keys_this_version_does_not_know() {
        let (_d, s) = store();
        // written by a newer version (or by hand): a key and a nested object this version has never heard of
        write_config(&s, r#"{"token":"keep-me","port":8765,"tiltDegrees":0.4,"futureFeature":{"on":true,"level":3},"note":"hand-written"}"#);
        let mut c = s.load().unwrap();
        c.show_progress = true; // a setting changed from the device
        s.save(&c).unwrap();
        let got: Value = serde_json::from_slice(&std::fs::read(s.config_file()).unwrap()).unwrap();
        assert_eq!((&got["token"], &got["showProgress"], &got["note"]), (&json!("keep-me"), &json!(true), &json!("hand-written")));
        assert_eq!(got["futureFeature"]["level"], json!(3), "an unknown key must survive a save: {got}");
        let again = s.load().unwrap();
        assert!(again.token == "keep-me" && again.show_progress);
        #[cfg(unix)]
        assert_eq!(mode(&s.config_file()), 0o600);
    }

    #[test]
    fn a_known_key_with_an_empty_value_is_ours_not_unknown() {
        // omitempty fields that are empty are absent from what we write, but must not be copied
        // back from the old file as "unknown" once the user clears them
        let (_d, s) = store();
        write_config(&s, r#"{"token":"t","port":8765,"tiltDegrees":0,"showProgress":true,"model":"haiku"}"#);
        let mut c = s.load().unwrap();
        assert!(c.unknown.is_empty(), "{:?}", c.unknown);
        c.show_progress = false;
        c.model = String::new();
        s.save(&c).unwrap();
        let got: Value = serde_json::from_slice(&std::fs::read(s.config_file()).unwrap()).unwrap();
        assert!(got.get("showProgress").is_none() && got.get("model").is_none(), "{got}");
    }

    fn load_ai(ai: &str) -> Result<Config, String> {
        let (_d, s) = store();
        write_config(&s, &format!(r#"{{"token":"t","port":8765,"tiltDegrees":0,"ai":{ai}}}"#));
        s.load()
    }

    #[test]
    fn without_ai_the_environment_is_told_and_no_tool_is_allowed() {
        // S1
        let (_d, s) = store();
        write_config(&s, r#"{"token":"t","port":8765,"tiltDegrees":0}"#);
        let c = s.load().unwrap();
        assert!(c.ai_environment() && c.tool_rules().is_empty());
        // S2
        let c = load_ai(r#"{"tools":{"allow":[]}}"#).unwrap();
        assert!(c.tool_rules().is_empty());
        assert!(!load_ai(r#"{"environment":false}"#).unwrap().ai_environment());
    }

    #[cfg(unix)]
    #[test]
    fn a_rule_is_kept_exactly_as_written() {
        // T2: the bridge validates and hands the rule on; it never rewrites it
        let c = load_ai(r#"{"tools":{"allow":["Bash(date)","Bash(ls:*)"]}}"#).unwrap();
        assert_eq!(c.tool_rules(), vec!["Bash(date)".to_string(), "Bash(ls:*)".to_string()]);
    }

    #[cfg(unix)]
    #[test]
    fn a_bad_rule_stops_the_start_and_says_which_one() {
        let long = format!("Bash({})", "a".repeat(260));
        let cases: Vec<(String, &str)> = vec![
            (r#"{"tools":{"allow":[""]}}"#.into(), "allow[0]"),                              // S3: empty
            (r#"{"tools":{"allow":[" Bash(date)"]}}"#.into(), "no leading"),                 // S3: space
            (r#"{"tools":{"allow":["Bash(date)\n"]}}"#.into(), "control"),                   // S3
            (format!(r#"{{"tools":{{"allow":["{long}"]}}}}"#), "characters"),                // S3: too long
            (r#"{"tools":{"allow":[3]}}"#.into(), "invalid type"),                           // S3: not a string
            (r#"{"tools":{"allow":["Read"]}}"#.into(), "only Bash"),                         // S4
            (r#"{"tools":{"allow":["WebFetch(domain:example.com)"]}}"#.into(), "only Bash"), // S4
            (r#"{"tools":{"allow":["Bash"]}}"#.into(), "only Bash"),                         // S5 (no pattern at all)
            (r#"{"tools":{"allow":["Bash()"]}}"#.into(), "allow everything"),                // S5
            (r#"{"tools":{"allow":["Bash(*)"]}}"#.into(), "allow everything"),               // S5
            (r#"{"tools":{"allow":["Bash(:*)"]}}"#.into(), "allow everything"),              // S5
            (r#"{"tools":{"allow":["*"]}}"#.into(), "only Bash"),                            // S5
            (r#"{"environment":"yes"}"#.into(), "invalid type"),                             // S9
            (r#"{"tools":{"deny":["Bash(rm:*)"]}}"#.into(), "unknown field"),                // S10
            (r#"{"tool":{"allow":["Bash(date)"]}}"#.into(), "unknown field"),                // S10: a misspelt key
        ];
        for (ai, want) in cases.into_iter() {
            let e = load_ai(&ai).unwrap_err();
            assert!(e.contains("config.json") && e.contains(want), "{ai} -> {e}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn more_than_32_rules_are_refused_and_repeats_are_dropped() {
        // S7
        let many: Vec<String> = (0..33).map(|i| format!("\"Bash(echo {i})\"")).collect();
        let e = load_ai(&format!(r#"{{"tools":{{"allow":[{}]}}}}"#, many.join(","))).unwrap_err();
        assert!(e.contains("at most 32"), "{e}");
        // S6
        let c = load_ai(r#"{"tools":{"allow":["Bash(date)","Bash(date)","Bash(pwd)"]}}"#).unwrap();
        assert_eq!(c.tool_rules(), vec!["Bash(date)".to_string(), "Bash(pwd)".to_string()]);
        assert_eq!(c.duplicate_tool_rules(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn tools_are_refused_on_windows() {
        // S8
        assert!(load_ai(r#"{"tools":{"allow":["Bash(date)"]}}"#).unwrap_err().contains("Windows"));
    }

    #[cfg(unix)]
    #[test]
    fn ai_survives_a_save() {
        let c = load_ai(r#"{"environment":false,"tools":{"allow":["Bash(date)"]}}"#).unwrap();
        let (_d, s) = store();
        std::fs::create_dir_all(s.dir()).unwrap();
        s.save(&c).unwrap();
        let got: Value = serde_json::from_slice(&std::fs::read(s.config_file()).unwrap()).unwrap();
        assert_eq!(got["ai"], json!({"environment": false, "tools": {"allow": ["Bash(date)"]}}));
    }
}

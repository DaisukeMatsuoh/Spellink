//! The bridge's boundary to "the AI": what is asked, and the one thing an AI has to be able to do
//! — answer, streaming text and showing signs of life (ADR 0015; Go: internal/provider). The rest
//! of the bridge (runs, page layout, memory, reconnection) does not know which AI is behind it.
//!
//! Authentication belongs to the provider and never leaves the machine: a CLI provider uses the
//! CLI's own login (the bridge never touches it); an API provider keeps its key in the bridge's
//! config file. The plugin on the device is told only the provider's display name.

use crate::history::{self, Turn};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub mod claudecode;

/// One question for the AI.
#[derive(Debug, Clone, Default)]
pub struct Request {
    /// Says how to answer (plain sentences, how long); it does not depend on the AI.
    pub system: String,
    /// The conversation so far in the same note, oldest first.
    pub earlier: Vec<Turn>,
    /// What was read from the handwriting.
    pub question: String,
    /// "ja" or "en", the language of the question.
    pub lang: String,
}

impl Request {
    /// The request as one piece of text, for a provider that is given a single prompt: the earlier
    /// conversation in front of the question.
    pub fn flat(&self) -> String {
        history::transcript(&self.earlier, &self.question, &self.lang)
    }
}

/// Where an answer goes as it is produced.
#[derive(Clone)]
pub struct Sink {
    /// Receives each piece of the answer text.
    pub text: Arc<dyn Fn(&str) + Send + Sync>,
    /// Reports a sign of life. `model` is false for "the process runs", true once the model itself
    /// has begun (thinking counts, not only visible text). A provider that cannot tell may call it
    /// only with true, from `text`.
    pub activity: Arc<dyn Fn(bool) + Send + Sync>,
    /// Reports a tool the AI used or was refused: one line for the owner's log (ADR 0022, T5).
    pub tool: Arc<dyn Fn(&str) + Send + Sync>,
}

impl Sink {
    /// A sink that drops everything.
    pub fn silent() -> Sink {
        Sink { text: Arc::new(|_| {}), activity: Arc::new(|_| {}), tool: Arc::new(|_| {}) }
    }
}

/// Why an answer did not come to an end.
#[derive(Debug, Clone, PartialEq)]
pub enum AskError {
    /// The caller cancelled; what had been produced so far.
    Cancelled(String),
    /// The AI failed; the message is for the owner (not for the plugin's user).
    Failed(String),
}

impl std::fmt::Display for AskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AskError::Cancelled(_) => f.write_str("cancelled"),
            AskError::Failed(m) => f.write_str(m),
        }
    }
}

pub type AskFuture<'a> = Pin<Box<dyn Future<Output = Result<String, AskError>> + Send + 'a>>;

/// Answers questions.
pub trait Provider: Send + Sync {
    /// Shown to the user as the connection's destination ("Claude Code").
    fn name(&self) -> String;
    /// Answers `r`, streaming through `s`, and returns the whole answer. It must return soon after
    /// `cancel` is cancelled. The caller owns the silence limits; the provider only reports activity.
    fn ask<'a>(&'a self, r: Request, s: Sink, cancel: CancellationToken) -> AskFuture<'a>;
}

/// Adapts a function to a provider; the function gets the flat text and the system prompt. It is
/// for tests and for providers that are a single function.
pub struct FuncProvider<F> {
    name: String,
    f: F,
}

pub fn func<F>(name: &str, f: F) -> FuncProvider<F>
where
    F: Fn(String, String, Sink, CancellationToken) -> AskFuture<'static> + Send + Sync,
{
    FuncProvider { name: name.to_string(), f }
}

impl<F> Provider for FuncProvider<F>
where
    F: Fn(String, String, Sink, CancellationToken) -> AskFuture<'static> + Send + Sync,
{
    fn name(&self) -> String {
        self.name.clone()
    }
    fn ask<'a>(&'a self, r: Request, s: Sink, cancel: CancellationToken) -> AskFuture<'a> {
        (self.f)(r.flat(), r.system, s, cancel)
    }
}

/// Asks for plain sentences of roughly `budget` characters (ja) or budget/5 words (en).
pub fn system_prompt(lang: &str, budget: i64) -> String {
    if lang == "en" {
        let words = 30.max(budget / 5);
        return [
            "You answer questions inside a handwritten notebook.".to_string(),
            "Your answer will be written onto the page as handwriting.".to_string(),
            "Use plain sentences only: no Markdown, headings, bullet symbols, tables, emoji or code blocks.".to_string(),
            format!("Be concise: aim for at most {words} words so the answer fits the space on the page. Use line breaks between paragraphs."),
            "The question was read by handwriting recognition and may contain recognition errors; infer the intent from context.".to_string(),
        ]
        .join("\n");
    }
    [
        "あなたは手書きノートの中で質問に答えるアシスタントです。".to_string(),
        "あなたの回答は手書きの文字としてノートのページに書き込まれます。".to_string(),
        "Markdown、見出し、箇条書き記号、表、絵文字、コードブロックは使わず、プレーンな文章で答えてください。".to_string(),
        format!("ページの余白に収まるよう、要点を簡潔に、{}字以内を目安に答えてください。段落を分けるときは改行を使ってください。", 60.max(budget)),
        "質問は手書き文字認識で読み取ったものなので、誤認識が含まれることがあります。文脈から意図を汲み取ってください。".to_string(),
    ]
    .join("\n")
}

/// Where and when the question is asked (ADR 0022, T4): the only things about the computer the AI is told.
#[derive(Debug, Clone, PartialEq)]
pub struct Now {
    /// ISO 8601 with the offset, e.g. 2026-11-28T10:00:00+09:00.
    pub iso: String,
    /// The time zone's name as the computer says it ("JST"; "UTC" when it cannot say).
    pub zone: String,
    /// "linux", "macos" or "windows".
    pub os: &'static str,
}

/// Reads `date '+%Y-%m-%dT%H:%M:%S %z %Z'` (another process's output is not trusted: the shape is
/// checked, anything else is `None`).
fn parse_date(out: &str) -> Option<(String, String)> {
    let mut p = out.split_whitespace();
    let (t, off, zone) = (p.next()?, p.next()?, p.next()?);
    let b = t.as_bytes();
    let shape = b.len() == 19
        && b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            10 => *c == b'T',
            13 | 16 => *c == b':',
            _ => c.is_ascii_digit(),
        });
    let o = off.as_bytes();
    let off_ok = o.len() == 5 && (o[0] == b'+' || o[0] == b'-') && o[1..].iter().all(u8::is_ascii_digit);
    let zone_ok = (1..=8).contains(&zone.len()) && zone.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '_'));
    (shape && off_ok && zone_ok && p.next().is_none()).then(|| (format!("{t}{}:{}", &off[..3], &off[3..]), zone.to_string()))
}

/// UTC from the clock, for a computer whose `date` cannot be asked.
fn utc_now() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    // days since 1970-01-01 -> civil date (Howard Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}+00:00", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// The local time, from the system's own `date` (the offset and the zone's name are the
/// computer's); UTC when that cannot be had.
pub async fn local_now() -> Now {
    let os = if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    };
    let asked = tokio::time::timeout(std::time::Duration::from_secs(2), tokio::process::Command::new("date").arg("+%Y-%m-%dT%H:%M:%S %z %Z").stdin(std::process::Stdio::null()).kill_on_drop(true).output()).await;
    let parsed = match asked {
        Ok(Ok(o)) if o.status.success() => parse_date(&String::from_utf8_lossy(&o.stdout)),
        _ => None,
    };
    match parsed {
        Some((iso, zone)) => Now { iso, zone, os },
        None => Now { iso: utc_now(), zone: "UTC".into(), os },
    }
}

/// The paragraph appended to the system prompt: the date, the zone and the kind of OS; and, when
/// tools are allowed, that they exist and should be mentioned when used. Nothing else about the
/// computer (no host or user name, path, address or setting) is ever put here.
pub fn environment_note(lang: &str, now: &Now, tools: bool) -> String {
    if lang == "en" {
        let mut s = format!("Current date and time: {} ({}). You run on the user's own computer ({}).", now.iso, now.zone, now.os);
        if tools {
            s.push_str(" You may use only the shell commands you have been allowed. When you use one, say so briefly in your answer.");
        }
        return s;
    }
    let mut s = format!("現在の日時: {}（{}）。あなたは利用者自身のパソコン（{}）で動いています。", now.iso, now.zone, now.os);
    if tools {
        s.push_str("許可されたシェルコマンドだけを使えます。使ったときは、答えの中でそれを一言添えてください。");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_output_is_checked_before_it_is_believed() {
        assert_eq!(parse_date("2026-11-28T10:00:00 +0900 JST\n"), Some(("2026-11-28T10:00:00+09:00".into(), "JST".into())));
        assert_eq!(parse_date("2026-11-28T10:00:00 -0330 -0330"), Some(("2026-11-28T10:00:00-03:30".into(), "-0330".into())));
        for bad in ["", "garbage", "2026-11-28 10:00:00 +0900 JST", "2026-11-28T10:00:00 +09 JST", "2026-11-28T10:00:00 +0900 J S T", "2026-11-28T10:00:00 +0900 JST extra", "2026-11-28T10:00:00 +0900 {{x}}"] {
            assert_eq!(parse_date(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_clock_fallback_is_a_valid_utc_time() {
        let (iso, _) = parse_date(&format!("{} +0000 UTC", &utc_now()[..19])).expect("shape");
        assert!(iso.ends_with("+00:00"));
    }

    #[tokio::test]
    async fn the_environment_note_holds_the_date_zone_and_os_and_nothing_else() {
        let now = local_now().await;
        assert!(parse_date(&format!("{} +0000 {}", &now.iso[..19], now.zone)).is_some(), "{now:?}");
        let fixed = Now { iso: "2026-11-28T10:00:00+09:00".into(), zone: "JST".into(), os: "linux" };
        let ja = environment_note("ja", &fixed, false);
        assert!(ja.contains("2026-11-28T10:00:00+09:00") && ja.contains("JST") && ja.contains("linux") && !ja.contains("シェル"), "{ja}");
        let en = environment_note("en", &fixed, true);
        assert!(en.contains("2026-11-28T10:00:00+09:00") && en.contains("say so briefly"), "{en}");
        // T4: nothing about this computer other than the three facts
        for text in [&ja, &en] {
            for private in [std::env::var("HOME").unwrap_or_default(), std::env::var("USER").unwrap_or_default()].iter().filter(|p| p.len() > 3) {
                assert!(!text.contains(private.as_str()), "{text}");
            }
        }
    }

    #[test]
    fn the_system_prompt_carries_the_length_budget() {
        assert!(system_prompt("ja", 300).contains("300字以内"));
        assert!(system_prompt("en", 300).contains("60 words"));
        assert!(system_prompt("ja", 1).contains("60字以内"), "the budget has a floor");
        assert!(system_prompt("en", 1).contains("30 words"), "and so does the English one");
    }

    #[test]
    fn flat_puts_the_earlier_conversation_first() {
        let r = Request { question: "next".into(), lang: "en".into(), earlier: vec![Turn { at: String::new(), q: "q1".into(), a: "a1".into() }], ..Default::default() };
        let flat = r.flat();
        assert!(flat.contains("q1") && flat.contains("a1") && flat.ends_with("next"), "{flat}");
        assert_eq!(Request { question: "alone".into(), ..Default::default() }.flat(), "alone", "without history the question is the whole prompt");
    }

    #[tokio::test]
    async fn func_adapts_a_function() {
        let p = func("test", |q, system, s, _| {
            Box::pin(async move {
                (s.text)("x");
                (s.activity)(true);
                Ok(format!("{q}|{system}"))
            })
        });
        assert_eq!(p.name(), "test");
        let got = p.ask(Request { system: "S".into(), question: "Q".into(), ..Default::default() }, Sink::silent(), CancellationToken::new()).await;
        assert_eq!(got, Ok("Q|S".to_string()));
    }
}

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
}

impl Sink {
    /// A sink that drops everything.
    pub fn silent() -> Sink {
        Sink { text: Arc::new(|_| {}), activity: Arc::new(|_| {}) }
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

#[cfg(test)]
mod tests {
    use super::*;

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

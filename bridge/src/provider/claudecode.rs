//! The provider that asks the locally installed, unmodified Claude Code CLI — signed in with the
//! user's own account — and streams the answer back. The bridge never touches credentials
//! (ADR 0005; Go: internal/provider/claudecode).

use super::{AskError, AskFuture, Provider, Request, Sink};
use serde::Deserialize;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio_util::sync::CancellationToken;

/// One line the CLI prints (`--output-format stream-json`); only what the bridge reads.
#[derive(Deserialize, Default)]
struct Event {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    is_error: bool,
    #[serde(default)]
    result: String,
    #[serde(default)]
    subtype: String,
    #[serde(default)]
    event: InnerEvent,
}

#[derive(Deserialize, Default)]
struct InnerEvent {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    delta: Delta,
}

#[derive(Deserialize, Default)]
struct Delta {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
}

/// The longest line of CLI output the bridge accepts.
const MAX_LINE: usize = 4 * 1024 * 1024;

pub struct Runner {
    pub executable: String,
    pub model: String,
    work_dir: PathBuf,
}

impl Runner {
    /// Prepares an empty working directory so the CLI never picks up a project's CLAUDE.md or files.
    pub fn new(executable: &str, model: &str) -> Result<Runner, String> {
        let dir = std::env::temp_dir().join(format!("spellink-bridge-{}-{:08x}", std::process::id(), rand::random::<u32>()));
        crate::util::create_private_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        Ok(Runner { executable: executable.to_string(), model: model.to_string(), work_dir: dir })
    }

    fn args(&self, system: &str) -> Vec<String> {
        let mut a: Vec<String> =
            ["-p", "--output-format", "stream-json", "--verbose", "--include-partial-messages", "--tools", "", "--strict-mcp-config", "--setting-sources", "", "--no-session-persistence", "--system-prompt"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        a.push(system.to_string());
        if !self.model.is_empty() {
            a.push("--model".into());
            a.push(self.model.clone());
        }
        a
    }

    /// Runs `claude -p` and streams the answer. The question goes in on standard input; the
    /// earlier conversation of the note is put in front of it. Signs of life: activity(false) for
    /// anything the process emits (it started), activity(true) once the model itself has begun
    /// (message and thinking events, not only visible text). A process that has started but whose
    /// model never answers — the network to the AI is down, the service is overloaded — is told
    /// apart from one that is working, long before any text exists.
    async fn run(&self, req: Request, sink: Sink, cancel: CancellationToken) -> Result<String, AskError> {
        let mut child = tokio::process::Command::new(&self.executable)
            .args(self.args(&req.system))
            .current_dir(&self.work_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // dropping the future (a cancelled or timed-out answer) must not leave the CLI running
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| AskError::Failed(format!("{}: {e}", self.executable)))?;

        let mut stdin = child.stdin.take().expect("piped");
        let question = req.flat();
        // The CLI reads all of stdin before it answers; writing from a task keeps a full pipe from
        // stalling us (and a CLI that exits early only closes it).
        tokio::spawn(async move {
            let _ = stdin.write_all(question.as_bytes()).await;
            let _ = stdin.shutdown().await;
        });
        let mut stderr = child.stderr.take().expect("piped");
        let stderr_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            let _ = (&mut stderr).take(1 << 20).read_to_end(&mut buf).await;
            buf
        });

        let mut answer = String::new();
        let mut result_err = String::new();
        let mut read_err: Option<String> = None;
        let mut lines = BufReader::new(child.stdout.take().expect("piped")).lines();
        loop {
            let line = tokio::select! {
                _ = cancel.cancelled() => {
                    let _ = child.kill().await;
                    return Err(AskError::Cancelled(answer));
                }
                l = lines.next_line() => l,
            };
            let line = match line {
                Ok(Some(l)) => l,
                Ok(None) => break,
                Err(e) => {
                    read_err = Some(e.to_string());
                    break;
                }
            };
            if line.len() > MAX_LINE {
                read_err = Some("a line of the CLI's output is too long".into());
                break;
            }
            let Ok(ev) = serde_json::from_str::<Event>(&line) else { continue };
            (sink.activity)(ev.kind == "stream_event" || ev.kind == "assistant" || ev.kind == "result");
            if ev.kind == "stream_event" && ev.event.kind == "content_block_delta" && ev.event.delta.kind == "text_delta" {
                answer.push_str(&ev.event.delta.text);
                (sink.text)(&ev.event.delta.text);
            } else if ev.kind == "result" && ev.is_error {
                result_err = if ev.result.is_empty() { ev.subtype } else { ev.result };
            }
        }
        if let Some(e) = read_err {
            let _ = child.kill().await;
            return Err(AskError::Failed(format!("claude: {e}")));
        }
        let status = tokio::select! {
            _ = cancel.cancelled() => {
                let _ = child.kill().await;
                return Err(AskError::Cancelled(answer));
            }
            s = child.wait() => s,
        };
        if cancel.is_cancelled() {
            return Err(AskError::Cancelled(answer));
        }
        if !result_err.is_empty() {
            return Err(AskError::Failed(result_err));
        }
        match status {
            Ok(s) if s.success() => Ok(answer),
            Ok(s) => {
                let buf = stderr_task.await.unwrap_or_default();
                let text = String::from_utf8_lossy(&buf);
                let tail: String = {
                    let t: Vec<char> = text.chars().collect();
                    t[t.len().saturating_sub(2000)..].iter().collect()
                };
                Err(AskError::Failed(format!("claude: {s}: {}", tail.trim())))
            }
            Err(e) => Err(AskError::Failed(format!("claude: {e}"))),
        }
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.work_dir);
    }
}

impl Provider for Runner {
    /// What the settings screen shows as the destination.
    fn name(&self) -> String {
        "Claude Code".into()
    }

    fn ask<'a>(&'a self, r: Request, s: Sink, cancel: CancellationToken) -> AskFuture<'a> {
        Box::pin(self.run(r, s, cancel))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::history::Turn;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// Writes a script that prints the given stream-json lines and exits with `code`.
    fn fake_claude(dir: &std::path::Path, lines: &[&str], code: i32) -> String {
        let path = dir.join("claude");
        let mut body = String::from("#!/bin/sh\ncat >/dev/null\n");
        for l in lines {
            body.push_str(&format!("printf '%s\\n' '{l}'\n"));
        }
        body.push_str(&format!("exit {code}\n"));
        script(&path, &body)
    }

    fn script(path: &std::path::Path, body: &str) -> String {
        std::fs::write(path, body).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().into_owned()
    }

    /// Starting a script that was written a moment ago can fail with "Text file busy" (ETXTBSY) when
    /// another test thread forks at that very time: a known Linux race between writing an
    /// executable and starting it. Only the tests write executables, and a start that failed this
    /// way never ran, so asking again is exact (no text, no sign of life was produced).
    async fn ask_script(r: &Runner, request: Request, sink: Sink, cancel: CancellationToken) -> Result<String, AskError> {
        for _ in 0..40 {
            match r.ask(request.clone(), sink.clone(), cancel.clone()).await {
                Err(AskError::Failed(m)) if m.contains("Text file busy") => tokio::time::sleep(Duration::from_millis(25)).await,
                other => return other,
            }
        }
        r.ask(request, sink, cancel).await
    }

    fn delta(text: &str) -> String {
        format!(r#"{{"type":"stream_event","event":{{"type":"content_block_delta","delta":{{"type":"text_delta","text":"{text}"}}}}}}"#)
    }

    fn req() -> Request {
        Request { system: "sys".into(), question: "q".into(), ..Default::default() }
    }

    fn sink_collecting() -> (Sink, Arc<Mutex<Vec<String>>>, Arc<Mutex<Vec<bool>>>) {
        let texts = Arc::new(Mutex::new(Vec::new()));
        let signs = Arc::new(Mutex::new(Vec::new()));
        let (t, s) = (texts.clone(), signs.clone());
        let sink = Sink { text: Arc::new(move |x| t.lock().unwrap().push(x.to_string())), activity: Arc::new(move |m| s.lock().unwrap().push(m)) };
        (sink, texts, signs)
    }

    #[tokio::test]
    async fn ask_streams_text_deltas() {
        let d = tempfile::tempdir().unwrap();
        let (d1, d2) = (delta("こん"), delta("にちは"));
        let exe = fake_claude(d.path(), &[&d1, r#"{"type":"system","subtype":"init"}"#, &d2, r#"{"type":"result","is_error":false}"#], 0);
        let r = Runner::new(&exe, "haiku").unwrap();
        let (sink, texts, _) = sink_collecting();
        let answer = ask_script(&r, req(), sink, CancellationToken::new()).await.unwrap();
        assert_eq!(answer, "こんにちは");
        assert_eq!(texts.lock().unwrap().join("|"), "こん|にちは");
    }

    #[tokio::test]
    async fn ask_reports_result_errors() {
        let d = tempfile::tempdir().unwrap();
        let exe = fake_claude(d.path(), &[r#"{"type":"result","is_error":true,"result":"Not logged in"}"#], 0);
        let r = Runner::new(&exe, "").unwrap();
        let e = ask_script(&r, req(), Sink::silent(), CancellationToken::new()).await.unwrap_err();
        assert_eq!(e, AskError::Failed("Not logged in".into()));
        let exe = fake_claude(d.path(), &[r#"{"type":"result","is_error":true,"subtype":"error_max_turns"}"#], 0);
        let r = Runner::new(&exe, "").unwrap();
        assert_eq!(ask_script(&r, req(), Sink::silent(), CancellationToken::new()).await.unwrap_err(), AskError::Failed("error_max_turns".into()), "no result text: the subtype says what happened");
    }

    #[tokio::test]
    async fn ask_reports_a_non_zero_exit_with_what_the_cli_said() {
        let d = tempfile::tempdir().unwrap();
        let exe = script(&d.path().join("claude"), "#!/bin/sh\ncat >/dev/null\necho 'boom happened' >&2\nexit 3\n");
        let r = Runner::new(&exe, "").unwrap();
        match ask_script(&r, req(), Sink::silent(), CancellationToken::new()).await {
            Err(AskError::Failed(m)) => assert!(m.contains("boom happened") && m.contains('3'), "{m}"),
            other => panic!("expected a failure for exit status 3, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_missing_cli_is_a_failure_that_names_it() {
        let r = Runner::new("/nonexistent/claude", "").unwrap();
        match ask_script(&r, req(), Sink::silent(), CancellationToken::new()).await {
            Err(AskError::Failed(m)) => assert!(m.contains("/nonexistent/claude"), "{m}"),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn ask_stops_quickly_when_cancelled_and_keeps_what_it_had() {
        let d = tempfile::tempdir().unwrap();
        let d1 = delta("途中");
        let exe = script(&d.path().join("claude"), &format!("#!/bin/sh\nprintf '%s\\n' '{d1}'\nsleep 30\n"));
        let r = Runner::new(&exe, "").unwrap();
        let cancel = CancellationToken::new();
        let c2 = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            c2.cancel();
        });
        let start = Instant::now();
        let e = ask_script(&r, req(), Sink::silent(), cancel).await.unwrap_err();
        assert!(start.elapsed() < Duration::from_secs(5), "cancel should stop the process quickly, took {:?}", start.elapsed());
        assert_eq!(e, AskError::Cancelled("途中".into()));
    }

    #[tokio::test]
    async fn dropping_the_answer_kills_the_cli() {
        let d = tempfile::tempdir().unwrap();
        let pid_file = d.path().join("pid");
        let exe = script(&d.path().join("claude"), &format!("#!/bin/sh\necho $$ > {}\nexec sleep 30\n", pid_file.display()));
        let r = Runner::new(&exe, "").unwrap();
        let fut = r.ask(req(), Sink::silent(), CancellationToken::new());
        // the caller's silence limit fires: the future is dropped. It is dropped once the CLI has
        // really started (it wrote its pid), however slow this machine is just now.
        let started = async {
            for _ in 0..600 {
                if std::fs::read_to_string(&pid_file).is_ok_and(|p| p.trim().parse::<i32>().is_ok()) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        };
        tokio::select! { r = fut => panic!("the CLI sleeps for 30 s: the answer cannot have ended: {:?}", r.map(|_| ())), _ = started => {} }
        let pid: i32 = std::fs::read_to_string(&pid_file).unwrap().trim().parse().unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists() || std::fs::read_to_string(format!("/proc/{pid}/stat")).map(|s| s.contains(") Z")).unwrap_or(true), "the CLI must not outlive the dropped answer");
    }

    #[tokio::test]
    async fn the_activity_tells_the_process_starting_from_the_model_working() {
        let d = tempfile::tempdir().unwrap();
        let d1 = delta("答え");
        let exe = fake_claude(
            d.path(),
            &[
                r#"{"type":"system","subtype":"init"}"#,
                r#"{"type":"system","subtype":"status"}"#,
                r#"{"type":"stream_event","event":{"type":"message_start"}}"#,
                r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"hm"}}}"#,
                &d1,
                r#"{"type":"result","is_error":false}"#,
            ],
            0,
        );
        let r = Runner::new(&exe, "").unwrap();
        let (sink, _, signs) = sink_collecting();
        ask_script(&r, req(), sink, CancellationToken::new()).await.unwrap();
        assert_eq!(*signs.lock().unwrap(), vec![false, false, true, true, true, true]);
    }

    #[tokio::test]
    async fn lines_that_are_not_events_are_no_sign_of_life() {
        let d = tempfile::tempdir().unwrap();
        let exe = fake_claude(d.path(), &["not json at all", r#"{"type":"system"}"#, r#"{"type":"result","is_error":false}"#], 0);
        let r = Runner::new(&exe, "").unwrap();
        let (sink, _, signs) = sink_collecting();
        ask_script(&r, req(), sink, CancellationToken::new()).await.unwrap();
        assert_eq!(signs.lock().unwrap().len(), 2, "a line that is not JSON is no sign of life");
    }

    #[tokio::test]
    async fn the_earlier_conversation_is_put_in_front_of_the_question() {
        let d = tempfile::tempdir().unwrap();
        let out = d.path().join("stdin.txt");
        let exe = script(&d.path().join("claude"), &format!("#!/bin/sh\ncat > {}\nprintf '%s\\n' '{{\"type\":\"result\",\"is_error\":false}}'\n", out.display()));
        let r = Runner::new(&exe, "").unwrap();
        let request = Request { system: "sys".into(), question: "それを3倍して".into(), lang: "ja".into(), earlier: vec![Turn { at: String::new(), q: "1+1は？".into(), a: "2です。".into() }] };
        ask_script(&r, request, Sink::silent(), CancellationToken::new()).await.unwrap();
        let got = std::fs::read_to_string(out).unwrap();
        for want in ["1+1は？", "2です。", "それを3倍して"] {
            assert!(got.contains(want), "the CLI must be given {want:?}, got {got:?}");
        }
    }

    #[tokio::test]
    async fn the_cli_is_started_with_the_arguments_the_bridge_relies_on() {
        let d = tempfile::tempdir().unwrap();
        let out = d.path().join("args.txt");
        let exe = script(
            &d.path().join("claude"),
            &format!("#!/bin/sh\ncat >/dev/null\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done > {}\npwd >> {}\nprintf '%s\\n' '{{\"type\":\"result\",\"is_error\":false}}'\n", out.display(), out.display()),
        );
        let r = Runner::new(&exe, "haiku").unwrap();
        ask_script(&r, req(), Sink::silent(), CancellationToken::new()).await.unwrap();
        let got = std::fs::read_to_string(out).unwrap();
        for want in ["-p", "stream-json", "--include-partial-messages", "--strict-mcp-config", "--no-session-persistence", "--system-prompt\nsys", "--model\nhaiku", "spellink-bridge-"] {
            assert!(got.contains(want), "missing {want:?} in:\n{got}");
        }
        assert!(got.contains("--tools\n\n--strict"), "the tools argument is an empty string: {got}");
    }

    #[test]
    fn it_is_a_provider_and_has_a_name() {
        let p: Box<dyn Provider> = Box::new(Runner::new("claude", "").unwrap());
        assert_eq!(p.name(), "Claude Code");
    }
}

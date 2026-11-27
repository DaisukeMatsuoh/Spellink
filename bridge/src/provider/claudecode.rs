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
    #[serde(default)]
    message: Message,
}

/// The part of an `assistant` / `user` event that tells what a tool was asked and what it said.
#[derive(Deserialize, Default)]
struct Message {
    /// Not always a list (a user message may be a string): anything else is ignored.
    #[serde(default, deserialize_with = "blocks")]
    content: Vec<Block>,
}

#[derive(Deserialize, Default)]
struct Block {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    input: serde_json::Value,
    #[serde(default)]
    content: serde_json::Value,
    #[serde(default)]
    is_error: bool,
}

fn blocks<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Block>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(v).unwrap_or_default())
}

/// The longest piece of a command or a tool's output that goes to the log (ADR 0022, X6).
const LOG_CHARS: usize = 500;

fn clip(s: &str) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > LOG_CHARS { one.chars().take(LOG_CHARS).collect::<String>() + "…" } else { one }
}

/// One line for the log for each tool use or result in the event.
fn tool_lines(m: &Message) -> Vec<String> {
    m.content
        .iter()
        .filter_map(|b| match b.kind.as_str() {
            "tool_use" => Some(format!("tool use: {} {}", b.name, clip(b.input.get("command").and_then(|c| c.as_str()).unwrap_or(&b.input.to_string())))),
            "tool_result" => {
                let text = match &b.content {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Array(a) => a.iter().filter_map(|x| x.get("text").and_then(|t| t.as_str())).collect::<Vec<_>>().join(" "),
                    _ => String::new(),
                };
                Some(format!("tool {}: {}", if b.is_error { "refused or failed" } else { "result" }, clip(&text)))
            }
            _ => None,
        })
        .collect()
}

/// Stops everything the AI started (T6). The CLI runs in its own process group, and the commands
/// it starts are in that group; when the answer ends any way but normally, the whole group is
/// killed. A normal end disarms it: the group's id could be given to another process by then.
struct Group {
    #[cfg(unix)]
    pgid: Option<i32>,
}

impl Group {
    fn of(child: &tokio::process::Child) -> Group {
        #[cfg(unix)]
        {
            Group { pgid: child.id().map(|p| p as i32) }
        }
        #[cfg(not(unix))]
        {
            let _ = child;
            Group {}
        }
    }
    /// A normal end: nothing is left to stop.
    fn disarm_if(&mut self, normal: bool) {
        #[cfg(unix)]
        if normal {
            self.pgid = None;
        }
        #[cfg(not(unix))]
        let _ = normal;
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(p) = self.pgid {
            // SAFETY: kill(2) with a negative pid signals the group; no memory is involved. A group
            // that is already gone only returns an error, which is the wanted state.
            unsafe {
                libc::kill(-p, libc::SIGKILL);
            }
        }
    }
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
    /// Tell the AI the date, zone and OS (ADR 0022).
    environment: bool,
    /// Allowed tool rules (validated by the config); tools are on only when this is not empty.
    rules: Vec<String>,
    /// The owner's standing instructions (trimmed; empty for none), added to the system prompt.
    instructions: String,
}

impl Runner {
    /// Prepares an empty working directory so the CLI never picks up a project's CLAUDE.md or files.
    pub fn new(executable: &str, model: &str) -> Result<Runner, String> {
        let dir = std::env::temp_dir().join(format!("spellink-bridge-{}-{:08x}", std::process::id(), rand::random::<u32>()));
        crate::util::create_private_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        Ok(Runner { executable: executable.to_string(), model: model.to_string(), work_dir: dir, environment: false, rules: Vec::new(), instructions: String::new() })
    }

    /// Applies the config's `ai` section (ADR 0022).
    pub fn with_ai(mut self, environment: bool, rules: Vec<String>, instructions: String) -> Runner {
        self.environment = environment;
        self.rules = rules;
        self.instructions = instructions;
        self
    }

    fn args(&self, system: &str) -> Vec<String> {
        let mut a: Vec<String> = ["-p", "--output-format", "stream-json", "--verbose", "--include-partial-messages"].iter().map(|s| s.to_string()).collect();
        // T1: tools only for a non-empty allow list; the rules go to the CLI as they are (T2)
        if self.rules.is_empty() {
            a.extend(["--tools".into(), String::new()]);
        } else {
            a.extend(["--tools".into(), "Bash".into(), "--allowedTools".into()]);
            a.extend(self.rules.iter().cloned());
        }
        a.extend(["--strict-mcp-config", "--setting-sources", "", "--no-session-persistence", "--system-prompt"].iter().map(|s| s.to_string()));
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
        let mut system = req.system.clone();
        if self.environment {
            let now = super::local_now().await;
            system = format!("{system}\n{}", super::environment_note(&req.lang, &now, !self.rules.is_empty()));
        }
        if !self.instructions.is_empty() {
            system = format!("{system}\n{}", super::instructions_note(&req.lang, &self.instructions));
        }
        let mut command = tokio::process::Command::new(&self.executable);
        #[cfg(unix)]
        command.process_group(0); // its own group, so the commands it starts can be stopped with it (T6)
        let mut child = command
            .args(self.args(&system))
            .current_dir(&self.work_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // dropping the future (a cancelled or timed-out answer) must not leave the CLI running
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| AskError::Failed(format!("{}: {e}", self.executable)))?;

        let mut group = Group::of(&child);
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
            // a tool's result arrives as a `user` event: between a tool use and its result the AI is
            // working, so that is a sign of life of the model too (X4)
            (sink.activity)(matches!(ev.kind.as_str(), "stream_event" | "assistant" | "user" | "result"));
            if ev.kind == "assistant" || ev.kind == "user" {
                for l in tool_lines(&ev.message) {
                    (sink.tool)(&l);
                }
            }
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
        group.disarm_if(status.as_ref().is_ok_and(|s| s.success()));
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
        let sink = Sink { text: Arc::new(move |x| t.lock().unwrap().push(x.to_string())), activity: Arc::new(move |m| s.lock().unwrap().push(m)), tool: Arc::new(|_| {}) };
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

    fn args_of(r: &Runner) -> Vec<String> {
        r.args("sys")
    }

    #[test]
    fn tools_are_on_only_for_a_non_empty_allow_list_and_the_rules_are_passed_unchanged() {
        // T1, S1, S2
        let off = Runner::new("claude", "").unwrap();
        let a = args_of(&off);
        assert!(a.windows(2).any(|w| w == ["--tools", ""]) && !a.contains(&"--allowedTools".to_string()), "{a:?}");
        let off = Runner::new("claude", "").unwrap().with_ai(true, vec![], String::new());
        assert!(!args_of(&off).contains(&"--allowedTools".to_string()), "an empty list keeps tools off");
        // T2: exactly what was given, in order
        let on = Runner::new("claude", "").unwrap().with_ai(true, vec!["Bash(date)".into(), "Bash(ls:*)".into()], String::new());
        let a = args_of(&on);
        let i = a.iter().position(|x| x == "--tools").unwrap();
        assert_eq!(a[i..i + 5], ["--tools", "Bash", "--allowedTools", "Bash(date)", "Bash(ls:*)"], "{a:?}");
        assert!(a.windows(2).any(|w| w == ["--system-prompt", "sys"]), "{a:?}");
    }

    #[tokio::test]
    async fn the_system_prompt_carries_the_environment_only_when_asked() {
        // T4 at the CLI: the note is appended to what the bridge said
        let d = tempfile::tempdir().unwrap();
        let out = d.path().join("args.txt");
        let exe =
            script(&d.path().join("claude"), &format!("#!/bin/sh\ncat >/dev/null\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done > {}\nprintf '%s\\n' '{{\"type\":\"result\",\"is_error\":false}}'\n", out.display()));
        let r = Runner::new(&exe, "").unwrap().with_ai(true, vec![], String::new());
        let request = Request { system: "sys".into(), question: "q".into(), lang: "ja".into(), ..Default::default() };
        ask_script(&r, request.clone(), Sink::silent(), CancellationToken::new()).await.unwrap();
        let got = std::fs::read_to_string(&out).unwrap();
        assert!(got.contains("現在の日時: 20") && got.contains("利用者自身のパソコン") && !got.contains("シェルコマンド"), "{got}");
        let r = Runner::new(&exe, "").unwrap().with_ai(false, vec!["Bash(date)".into()], String::new());
        ask_script(&r, request, Sink::silent(), CancellationToken::new()).await.unwrap();
        let got = std::fs::read_to_string(&out).unwrap();
        assert!(!got.contains("現在の日時") && got.contains("--allowedTools\nBash(date)"), "{got}");
    }

    #[tokio::test]
    async fn a_tool_use_and_its_result_are_signs_of_life_and_are_logged_and_a_refusal_is_told() {
        // X1, X2, X4, X6
        let d = tempfile::tempdir().unwrap();
        let big = "x".repeat(2000);
        let use_ = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"date"}}]}}"#;
        let ok = format!(r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","content":"{big}"}}]}}}}"#);
        let denied = r#"{"type":"user","message":{"content":[{"type":"tool_result","is_error":true,"content":[{"type":"text","text":"Permission to use Bash has been denied"}]}]}}"#;
        let plain_user = r#"{"type":"user","message":{"content":"just a string"}}"#;
        let exe = fake_claude(d.path(), &[use_, &ok, denied, plain_user, r#"{"type":"result","is_error":false}"#], 0);
        let r = Runner::new(&exe, "").unwrap();
        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let signs = Arc::new(Mutex::new(Vec::<bool>::new()));
        let (l2, s2) = (lines.clone(), signs.clone());
        let sink = Sink { text: Arc::new(|_| {}), activity: Arc::new(move |m| s2.lock().unwrap().push(m)), tool: Arc::new(move |l| l2.lock().unwrap().push(l.to_string())) };
        ask_script(&r, req(), sink, CancellationToken::new()).await.unwrap();
        assert_eq!(*signs.lock().unwrap(), vec![true; 5], "tool events count as the model working");
        let got = lines.lock().unwrap().clone();
        assert_eq!(got.len(), 3, "{got:?}");
        assert_eq!(got[0], "tool use: Bash date");
        assert!(got[1].starts_with("tool result: xxx") && got[1].chars().count() < 520, "X6: only the start is logged: {}", got[1].len());
        assert_eq!(got[2], "tool refused or failed: Permission to use Bash has been denied");
    }

    /// The pid a script wrote, once it has.
    async fn wait_pid(f: &std::path::Path) -> i32 {
        for _ in 0..600 {
            if let Some(p) = std::fs::read_to_string(f).ok().and_then(|p| p.trim().parse::<i32>().ok()) {
                return p;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("the script never wrote its pid");
    }

    fn gone(pid: i32) -> bool {
        !std::path::Path::new(&format!("/proc/{pid}")).exists() || std::fs::read_to_string(format!("/proc/{pid}/stat")).map(|s| s.contains(") Z")).unwrap_or(true)
    }

    fn with_child(d: &std::path::Path) -> (String, std::path::PathBuf) {
        let pid_file = d.join("child.pid");
        // the CLI starts a command (here: sleep) and waits for it, like a tool the AI asked for
        let exe = script(&d.join("claude"), &format!("#!/bin/sh\nsleep 30 &\necho $! > {}\nwait\n", pid_file.display()));
        (exe, pid_file)
    }

    #[tokio::test]
    async fn cancelling_stops_the_commands_the_ai_started_too() {
        // T6, X5
        let d = tempfile::tempdir().unwrap();
        let (exe, pid_file) = with_child(d.path());
        let r = Runner::new(&exe, "").unwrap();
        let cancel = CancellationToken::new();
        let c2 = cancel.clone();
        let pf = pid_file.clone();
        tokio::spawn(async move {
            wait_pid(&pf).await;
            c2.cancel();
        });
        let e = ask_script(&r, req(), Sink::silent(), cancel).await.unwrap_err();
        assert!(matches!(e, AskError::Cancelled(_)), "{e:?}");
        let pid = wait_pid(&pid_file).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(gone(pid), "the command the CLI started must not outlive the cancelled answer");
    }

    #[tokio::test]
    async fn a_timeout_that_drops_the_answer_stops_the_commands_too() {
        // T6, X3 (the caller's limit drops the future)
        let d = tempfile::tempdir().unwrap();
        let (exe, pid_file) = with_child(d.path());
        let r = Runner::new(&exe, "").unwrap();
        let fut = r.ask(req(), Sink::silent(), CancellationToken::new());
        tokio::select! { r = fut => panic!("it cannot end: {:?}", r.map(|_| ())), _ = wait_pid(&pid_file) => {} }
        let pid = wait_pid(&pid_file).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(gone(pid), "the command must not outlive the dropped answer");
    }

    #[tokio::test]
    async fn the_owners_standing_instructions_are_added_after_the_environment() {
        let d = tempfile::tempdir().unwrap();
        let out = d.path().join("args.txt");
        let exe =
            script(&d.path().join("claude"), &format!("#!/bin/sh\ncat >/dev/null\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done > {}\nprintf '%s\\n' '{{\"type\":\"result\",\"is_error\":false}}'\n", out.display()));
        let r = Runner::new(&exe, "").unwrap().with_ai(true, vec![], "私は大輔。敬語は不要。".into());
        let request = Request { system: "sys".into(), question: "q".into(), lang: "ja".into(), ..Default::default() };
        ask_script(&r, request.clone(), Sink::silent(), CancellationToken::new()).await.unwrap();
        let got = std::fs::read_to_string(&out).unwrap();
        let (env, own) = (got.find("現在の日時").unwrap(), got.find("私は大輔。敬語は不要。").unwrap());
        assert!(env < own && got.contains("利用者からの指示"), "{got}");
        let r = Runner::new(&exe, "").unwrap().with_ai(false, vec![], String::new());
        ask_script(&r, request, Sink::silent(), CancellationToken::new()).await.unwrap();
        assert!(!std::fs::read_to_string(&out).unwrap().contains("利用者からの指示"));
    }
}

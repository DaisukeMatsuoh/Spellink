//! spellink-bridge-rs — a measurement prototype of the bridge's core in Rust (see README.md).
//!
//!   compose  --glyphs F --layout JSON [--seed N] [--slope S] [--full F] [--half H]
//!            stdin: one JSON string per line (the pieces of text as they stream in)
//!            stdout: one JSON line per finished line of handwriting, then a summary line
//!   ask      --glyphs F --layout JSON [--claude PATH] [--model M] QUESTION
//!            runs the official CLI like the Go bridge does and composes as the answer streams
//!   bench    --glyphs F --layout JSON --text FILE [--iterations N]
//!   idle     --glyphs F [--seconds S]     load the glyphs and wait, to measure resident memory

mod glyphs;
mod handwriting;
mod layout;
mod random;

use layout::{Composer, Layout};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, ExitCode, Stdio};
use std::time::Instant;

fn args() -> (String, HashMap<String, String>, Vec<String>) {
    let mut it = std::env::args().skip(1);
    let cmd = it.next().unwrap_or_default();
    let mut flags = HashMap::new();
    let mut rest = Vec::new();
    while let Some(a) = it.next() {
        if let Some(name) = a.strip_prefix("--") {
            flags.insert(name.to_string(), it.next().unwrap_or_default());
        } else {
            rest.push(a);
        }
    }
    (cmd, flags, rest)
}

fn flag_f64(flags: &HashMap<String, String>, name: &str) -> Result<f64, String> {
    match flags.get(name) {
        None => Ok(0.0),
        Some(v) => v.parse().map_err(|_| format!("--{name}: not a number: {v}")),
    }
}

fn load(flags: &HashMap<String, String>) -> Result<(glyphs::Font, Layout, u32), String> {
    let path = flags.get("glyphs").ok_or("--glyphs is required")?;
    let file = std::fs::File::open(path).map_err(|e| format!("{path}: {e}"))?;
    let font = glyphs::read_font(BufReader::new(file)).map_err(|e| format!("{path}: {e}"))?;
    let mut layout: Layout = match flags.get("layout") {
        Some(j) => serde_json::from_str(j).map_err(|e| format!("--layout: {e}"))?,
        None => Layout::default(),
    };
    layout.slope = flag_f64(flags, "slope")?;
    layout.spacing_full = flag_f64(flags, "full")?;
    layout.spacing_half = flag_f64(flags, "half")?;
    let seed = match flags.get("seed") {
        None => 0,
        Some(s) => s.parse().map_err(|_| format!("--seed: not a 32-bit number: {s}"))?,
    };
    Ok((font, layout, seed))
}

fn print_lines(out: &mut impl Write, lines: &[layout::Line]) -> Result<(), String> {
    for l in lines {
        serde_json::to_writer(&mut *out, l).map_err(|e| e.to_string())?;
        out.write_all(b"\n").map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn summary(out: &mut impl Write, c: &Composer) -> Result<(), String> {
    let missing: Vec<String> = c.missing.iter().map(|c| c.to_string()).collect();
    let s = serde_json::json!({"summary": true, "missing": missing, "truncated": c.truncated, "pages": c.pages, "capacity": layout::capacity(c.layout())});
    writeln!(out, "{s}").map_err(|e| e.to_string())
}

fn compose(flags: &HashMap<String, String>) -> Result<(), String> {
    let (font, layout, seed) = load(flags)?;
    let mut c = Composer::new(&font, layout, seed);
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    for line in std::io::stdin().lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let piece: String = serde_json::from_str(&line).map_err(|e| format!("input: {e}"))?;
        print_lines(&mut out, &c.push(&piece))?;
    }
    print_lines(&mut out, &c.finish())?;
    summary(&mut out, &c)
}

/// The same CLI invocation as bridge/internal/provider/claudecode, parsing the same events.
fn ask(flags: &HashMap<String, String>, question: &str) -> Result<(), String> {
    let (font, layout, seed) = load(flags)?;
    let exe = flags.get("claude").map(String::as_str).unwrap_or("claude");
    let dir = std::env::temp_dir().join(format!("spellink-bridge-rs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut cmd = Command::new(exe);
    cmd.args(["-p", "--output-format", "stream-json", "--verbose", "--include-partial-messages",
        "--tools", "", "--strict-mcp-config", "--setting-sources", "", "--no-session-persistence",
        "--system-prompt", "あなたは手書きノートの中で質問に答えるアシスタントです。プレーンな文章で、簡潔に答えてください。"]);
    if let Some(m) = flags.get("model") {
        cmd.args(["--model", m]);
    }
    let started = Instant::now();
    let mut child = cmd.current_dir(&dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|e| format!("{exe}: {e}"))?;
    child.stdin.take().unwrap().write_all(question.as_bytes()).map_err(|e| e.to_string())?;
    let mut c = Composer::new(&font, layout, seed);
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut first: Option<u128> = None;
    let mut result_err: Option<String> = None;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let line = line.map_err(|e| e.to_string())?;
        let Ok(ev) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        let typ = ev["type"].as_str().unwrap_or("");
        if typ == "stream_event"
            && ev["event"]["type"] == "content_block_delta"
            && ev["event"]["delta"]["type"] == "text_delta"
        {
            let text = ev["event"]["delta"]["text"].as_str().unwrap_or("");
            let lines = c.push(text);
            if first.is_none() && !lines.is_empty() {
                first = Some(started.elapsed().as_millis());
            }
            print_lines(&mut out, &lines)?;
        } else if typ == "result" && ev["is_error"].as_bool() == Some(true) {
            result_err = Some(ev["result"].as_str().unwrap_or("error").to_string());
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&dir);
    if let Some(e) = result_err {
        return Err(e);
    }
    if !status.success() {
        return Err(format!("{exe} exited with {status}"));
    }
    let last = c.finish();
    if first.is_none() && !last.is_empty() {
        first = Some(started.elapsed().as_millis());
    }
    print_lines(&mut out, &last)?;
    match first {
        Some(ms) => eprintln!("first line after {ms} ms, done after {} ms", started.elapsed().as_millis()),
        None => eprintln!("no line was written, done after {} ms", started.elapsed().as_millis()),
    }
    summary(&mut out, &c)
}

fn bench(flags: &HashMap<String, String>) -> Result<(), String> {
    let (font, layout, seed) = load(flags)?;
    let path = flags.get("text").ok_or("--text is required")?;
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let n: u32 = flags.get("iterations").map(|s| s.parse().unwrap_or(100)).unwrap_or(100);
    let started = Instant::now();
    let mut strokes = 0usize;
    for i in 0..n {
        let mut c = Composer::new(&font, layout.clone(), seed.wrapping_add(i));
        // pieces of ~8 characters, as the CLI streams them
        let chars: Vec<char> = text.chars().collect();
        for chunk in chars.chunks(8) {
            let piece: String = chunk.iter().collect();
            for l in c.push(&piece) {
                strokes += l.strokes.len();
            }
        }
        for l in c.finish() {
            strokes += l.strokes.len();
        }
    }
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    println!("{{\"impl\":\"rust\",\"iterations\":{n},\"ms\":{ms:.1},\"msPerAnswer\":{:.3},\"strokes\":{strokes}}}", ms / n as f64);
    Ok(())
}

fn idle(flags: &HashMap<String, String>) -> Result<(), String> {
    let (font, _, _) = load(flags)?;
    let secs: u64 = flags.get("seconds").map(|s| s.parse().unwrap_or(5)).unwrap_or(5);
    eprintln!("loaded {} glyphs from {} source(s); waiting {secs} s", font.glyphs.len(), font.sources.len());
    std::thread::sleep(std::time::Duration::from_secs(secs));
    Ok(())
}

fn main() -> ExitCode {
    let (cmd, flags, rest) = args();
    let r = match cmd.as_str() {
        "compose" => compose(&flags),
        "ask" => ask(&flags, &rest.join(" ")),
        "bench" => bench(&flags),
        "idle" => idle(&flags),
        _ => Err("usage: spellink-bridge-rs compose|ask|bench|idle (see src/main.rs)".to_string()),
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Run the same compose as the Go bridge on the recorded input and get the same bytes. The
    /// expected file was produced by `spellink-bridge compose` (Go) with the flags below; to
    /// record it again, see README.md.
    #[test]
    fn writes_exactly_what_the_go_bridge_writes() {
        let font = glyphs::read_font(fs::File::open("testdata/glyphs.bin").unwrap()).unwrap();
        let mut layout: Layout = serde_json::from_str(&fs::read_to_string("testdata/layout.json").unwrap()).unwrap();
        layout.slope = -0.005;
        layout.spacing_full = 1.1;
        layout.spacing_half = 1.2;
        let mut c = Composer::new(&font, layout, 777);
        let mut out = Vec::new();
        for line in fs::read_to_string("testdata/input.jsonl").unwrap().lines() {
            let piece: String = serde_json::from_str(line).unwrap();
            print_lines(&mut out, &c.push(&piece)).unwrap();
        }
        print_lines(&mut out, &c.finish()).unwrap();
        summary(&mut out, &c).unwrap();
        let want = fs::read_to_string("testdata/expected-go.jsonl").unwrap();
        let got = String::from_utf8(out).unwrap();
        for (i, (g, w)) in got.lines().zip(want.lines()).enumerate() {
            assert_eq!(g, w, "line {i} differs from the Go bridge");
        }
        assert_eq!(got.lines().count(), want.lines().count());
    }

    #[test]
    fn mulberry32_matches_the_reference_values() {
        // mulberry32(0): the first outputs of the Go bridge (bridge/internal/ink/random.go), ×2^32
        let mut r = random::Random::new(0);
        let got: Vec<u64> = (0..3).map(|_| (r.float() * 4294967296.0) as u64).collect();
        assert_eq!(got, vec![1144304738, 1416247, 958946056]);
    }

    #[test]
    fn a_file_that_is_not_a_glyph_file_is_refused() {
        assert!(matches!(glyphs::read_font(&b"not gzip"[..]), Err(glyphs::FontError::Format)));
    }
}

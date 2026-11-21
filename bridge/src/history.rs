//! Remembers the questions and answers of each note so a follow-up written in the same note
//! continues the conversation (ADR 0012; Go: internal/history). The AI is started fresh for every
//! question, so the bridge keeps the transcript and hands the most recent part back each time.
//!
//! Everything is appended to one JSON-lines file per note and never rewritten; only what is handed
//! to the model is limited (by size, not by number of turns).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

/// One question (as read from the handwriting) and the answer that was written back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Turn {
    /// RFC 3339, UTC.
    #[serde(default)]
    pub at: String,
    #[serde(default)]
    pub q: String,
    #[serde(default)]
    pub a: String,
}

impl Turn {
    fn size(&self) -> usize {
        self.q.chars().count() + self.a.chars().count()
    }
}

/// One file per note under `dir`. A store with no directory (`Store::none()`) remembers nothing.
pub struct Store {
    dir: Option<PathBuf>,
    mu: Mutex<()>,
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Store {
        Store { dir: Some(dir.into()), mu: Mutex::new(()) }
    }

    /// A store that remembers nothing.
    pub fn none() -> Store {
        Store { dir: None, mu: Mutex::new(()) }
    }

    /// Names a note's transcript by a hash of its key, so the key (a file path on the device)
    /// never becomes part of a path on this machine.
    fn file(dir: &std::path::Path, note: &str) -> PathBuf {
        let h = Sha256::digest(note.as_bytes());
        dir.join(format!("{}.jsonl", hex::encode(&h[..8])))
    }

    /// Records a finished turn.
    pub fn append(&self, note: &str, t: &Turn) -> Result<(), String> {
        let Some(dir) = &self.dir else { return Ok(()) };
        if note.is_empty() {
            return Ok(());
        }
        let _g = self.mu.lock().unwrap();
        crate::util::create_private_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let file = Self::file(dir, note);
        let mut opts = std::fs::OpenOptions::new();
        opts.create(true).append(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        let mut f = opts.open(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        let mut line = serde_json::to_vec(t).map_err(|e| e.to_string())?;
        line.push(b'\n');
        f.write_all(&line).map_err(|e| format!("{}: {e}", file.display()))
    }

    /// The newest turns of a note, oldest first, as many as fit in `max_chars` (characters of
    /// questions plus answers). A turn is never cut in half. Damaged lines (a power cut in the
    /// middle of a write) are skipped.
    pub fn recent(&self, note: &str, max_chars: i64) -> Result<Vec<Turn>, String> {
        let Some(dir) = &self.dir else { return Ok(vec![]) };
        if note.is_empty() || max_chars <= 0 {
            return Ok(vec![]);
        }
        let _g = self.mu.lock().unwrap();
        let file = Self::file(dir, note);
        let bytes = match std::fs::read(&file) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(format!("{}: {e}", file.display())),
        };
        let all: Vec<Turn> = bytes.split(|b| *b == b'\n').filter(|l| l.len() <= 8 << 20).filter_map(|l| serde_json::from_slice::<Turn>(l).ok()).collect();
        let (mut used, mut from) = (0usize, all.len());
        while from > 0 && used + all[from - 1].size() <= max_chars as usize {
            from -= 1;
            used += all[from].size();
        }
        Ok(all[from..].to_vec())
    }

    /// Forgets one note's conversation; returns how many notes had one (0 or 1).
    pub fn clear_note(&self, note: &str) -> Result<usize, String> {
        let Some(dir) = &self.dir else { return Ok(0) };
        if note.is_empty() {
            return Ok(0);
        }
        let _g = self.mu.lock().unwrap();
        let file = Self::file(dir, note);
        match std::fs::remove_file(&file) {
            Ok(()) => Ok(1),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
            Err(e) => Err(format!("{}: {e}", file.display())),
        }
    }

    /// Forgets every note's conversation; returns how many notes had one.
    pub fn clear(&self) -> Result<usize, String> {
        let Some(dir) = &self.dir else { return Ok(0) };
        let _g = self.mu.lock().unwrap();
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(format!("{}: {e}", dir.display())),
        };
        let mut n = 0;
        for entry in entries {
            let path = entry.map_err(|e| format!("{}: {e}", dir.display()))?.path();
            if path.extension().is_some_and(|x| x == "jsonl") {
                std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                n += 1;
            }
        }
        Ok(n)
    }
}

/// Puts the earlier turns in front of the new question as the text the AI reads. With no earlier
/// turns it is the question itself.
pub fn transcript(turns: &[Turn], question: &str, lang: &str) -> String {
    if turns.is_empty() {
        return question.to_string();
    }
    let mut b = String::new();
    if lang == "en" {
        b.push_str("Earlier in this notebook (oldest first). The new question may refer to it:\n\n");
        for t in turns {
            b.push_str(&format!("[Question] {}\n[Answer] {}\n\n", t.q, t.a));
        }
        b.push_str(&format!("---\nNew question (answer this one):\n{question}"));
        return b;
    }
    b.push_str("このノートでのこれまでのやり取り（古い順）です。新しい質問はこれを踏まえている場合があります。\n\n");
    for t in turns {
        b.push_str(&format!("[質問] {}\n[回答] {}\n\n", t.q, t.a));
    }
    b.push_str(&format!("---\n今回の質問（これに答えてください）:\n{question}"));
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(q: &str, a: &str) -> Turn {
        Turn { at: "1970-01-01T00:00:00Z".into(), q: q.into(), a: a.into() }
    }

    fn store() -> (tempfile::TempDir, Store) {
        let d = tempfile::tempdir().unwrap();
        let s = Store::new(d.path().join("history"));
        (d, s)
    }

    #[test]
    fn recent_keeps_the_newest_within_the_budget_and_oldest_first() {
        let (_d, s) = store();
        for q in ["a", "b", "c"] {
            s.append("note", &turn(q, &"x".repeat(9))).unwrap(); // 10 chars each
        }
        let got = s.recent("note", 25).unwrap();
        assert_eq!(got.iter().map(|t| t.q.as_str()).collect::<Vec<_>>(), ["b", "c"]);
        assert!(s.recent("note", 5).unwrap().is_empty(), "a turn larger than the budget must not be cut");
    }

    #[test]
    fn the_budget_counts_characters_not_bytes() {
        let (_d, s) = store();
        s.append("n", &turn("あいう", "えお")).unwrap(); // 5 characters, 15 bytes
        assert_eq!(s.recent("n", 5).unwrap().len(), 1);
        assert_eq!(s.recent("n", 4).unwrap().len(), 0);
    }

    #[test]
    fn notes_are_separate_and_unknown_notes_are_empty() {
        let (_d, s) = store();
        s.append("one", &turn("q1", "a1")).unwrap();
        s.append("two", &turn("q2", "a2")).unwrap();
        let one = s.recent("one", 1000).unwrap();
        assert!(one.len() == 1 && one[0].q == "q1");
        assert!(s.recent("never", 1000).unwrap().is_empty());
    }

    #[test]
    fn it_survives_a_restart_and_skips_damaged_lines() {
        let (d, first) = store();
        first.append("n", &turn("q1", "a1")).unwrap();
        // a half-written line (e.g. power cut) must not lose the other turns
        let file = Store::file(&d.path().join("history"), "n");
        let mut f = std::fs::OpenOptions::new().append(true).open(file).unwrap();
        f.write_all(b"{\"q\":\"torn\n").unwrap();
        drop(f);
        first.append("n", &turn("q2", "a2")).unwrap();
        let got = Store::new(d.path().join("history")).recent("n", 1000).unwrap();
        assert!(got.len() == 2 && got[1].q == "q2", "{got:?}");
    }

    #[test]
    fn the_note_key_never_becomes_part_of_a_path() {
        let (d, s) = store();
        s.append("/storage/emulated/0/Note/secret name.note", &turn("q", "a")).unwrap();
        let names: Vec<String> = std::fs::read_dir(d.path().join("history")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(names.len() == 1 && !names[0].contains("secret") && names[0].ends_with(".jsonl"), "{names:?}");
    }

    #[test]
    fn clear_forgets_everything() {
        let (_d, s) = store();
        s.append("a", &turn("q", "a")).unwrap();
        s.append("b", &turn("q", "a")).unwrap();
        assert_eq!(s.clear().unwrap(), 2);
        assert!(s.recent("a", 1000).unwrap().is_empty());
        assert_eq!(s.clear().unwrap(), 0, "clearing nothing is fine, and so is a directory that does not exist yet");
    }

    #[test]
    fn a_store_with_no_directory_and_an_empty_note_remember_nothing() {
        let none = Store::none();
        none.append("n", &turn("q", "a")).unwrap();
        assert!(none.recent("n", 100).unwrap().is_empty());
        let (_d, real) = store();
        real.append("", &turn("q", "a")).unwrap();
        assert!(real.recent("", 100).unwrap().is_empty(), "an empty note key is not stored");
        assert!(!_d.path().join("history").exists(), "and nothing was created for it");
    }

    #[test]
    fn transcript_puts_the_question_last() {
        assert_eq!(transcript(&[], "今日は？", "ja"), "今日は？", "without history the question is sent as is");
        let got = transcript(&[turn("1+1は？", "2です。")], "それを3倍して", "ja");
        for want in ["[質問] 1+1は？", "[回答] 2です。", "今回の質問", "それを3倍して"] {
            assert!(got.contains(want), "missing {want:?} in {got:?}");
        }
        assert!(got.ends_with("それを3倍して"));
        let en = transcript(&[turn("q", "a")], "next", "en");
        assert!(en.contains("[Question] q") && en.ends_with("next"), "{en}");
    }

    #[test]
    fn clear_note_forgets_only_that_note() {
        let (_d, s) = store();
        s.append("a", &turn("qa", "aa")).unwrap();
        s.append("b", &turn("qb", "ab")).unwrap();
        assert_eq!(s.clear_note("a").unwrap(), 1);
        assert!(s.recent("a", 1000).unwrap().is_empty());
        assert_eq!(s.recent("b", 1000).unwrap().len(), 1, "b must be untouched");
        assert_eq!(s.clear_note("never").unwrap(), 0, "a note with no conversation");
        assert_eq!(s.clear_note("").unwrap(), 0, "no key, nothing to forget");
        assert_eq!(Store::none().clear_note("a").unwrap(), 0, "a store with no directory remembers nothing");
    }
}

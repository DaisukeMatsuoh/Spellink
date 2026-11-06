// Package history remembers the questions and answers of each note so a follow-up written in the
// same note continues the conversation (ADR 0012). Claude Code is started fresh for every
// question, so the bridge keeps the transcript and hands the most recent part back each time.
//
// Everything is appended to one JSON-lines file per note and never rewritten; only what is
// handed to the model is limited (by size, not by number of turns).
package history

import (
	"bufio"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"
	"unicode/utf8"
)

// Turn is one question (as read from the handwriting) and the answer that was written back.
type Turn struct {
	At time.Time `json:"at"`
	Q  string    `json:"q"`
	A  string    `json:"a"`
}

func (t Turn) size() int { return utf8.RuneCountInString(t.Q) + utf8.RuneCountInString(t.A) }

// Store keeps one file per note under Dir. The zero value is not usable; a nil *Store is, and
// remembers nothing.
type Store struct {
	Dir string
	mu  sync.Mutex
}

// file names a note's transcript by a hash of its key, so the key (a file path on the device)
// never becomes part of a path on this machine.
func (s *Store) file(note string) string {
	h := sha256.Sum256([]byte(note))
	return filepath.Join(s.Dir, hex.EncodeToString(h[:8])+".jsonl")
}

// Append records a finished turn.
func (s *Store) Append(note string, t Turn) error {
	if s == nil || note == "" {
		return nil
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if err := os.MkdirAll(s.Dir, 0o700); err != nil {
		return err
	}
	f, err := os.OpenFile(s.file(note), os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o600)
	if err != nil {
		return err
	}
	defer f.Close()
	return json.NewEncoder(f).Encode(t)
}

// Recent returns the newest turns of a note, oldest first, as many as fit in maxChars
// (characters of questions plus answers). A turn is never cut in half.
func (s *Store) Recent(note string, maxChars int) ([]Turn, error) {
	if s == nil || note == "" || maxChars <= 0 {
		return nil, nil
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	f, err := os.Open(s.file(note))
	if errors.Is(err, fs.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	defer f.Close()
	var all []Turn
	sc := bufio.NewScanner(f)
	sc.Buffer(make([]byte, 0, 64<<10), 8<<20)
	for sc.Scan() {
		var t Turn
		if json.Unmarshal(sc.Bytes(), &t) == nil {
			all = append(all, t)
		}
	}
	if err := sc.Err(); err != nil {
		return nil, err
	}
	used, from := 0, len(all)
	for from > 0 && used+all[from-1].size() <= maxChars {
		from--
		used += all[from].size()
	}
	return all[from:], nil
}

// Clear forgets every note's conversation and returns how many notes had one.
func (s *Store) Clear() (int, error) {
	if s == nil {
		return 0, nil
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	files, err := filepath.Glob(filepath.Join(s.Dir, "*.jsonl"))
	if err != nil {
		return 0, err
	}
	n := 0
	for _, f := range files {
		if err := os.Remove(f); err != nil {
			return n, err
		}
		n++
	}
	return n, nil
}

// Transcript puts the earlier turns in front of the new question as the text Claude Code reads.
// With no earlier turns it is the question itself.
func Transcript(turns []Turn, question, lang string) string {
	if len(turns) == 0 {
		return question
	}
	var b strings.Builder
	if lang == "en" {
		b.WriteString("Earlier in this notebook (oldest first). The new question may refer to it:\n\n")
		for _, t := range turns {
			b.WriteString("[Question] " + t.Q + "\n[Answer] " + t.A + "\n\n")
		}
		b.WriteString("---\nNew question (answer this one):\n" + question)
		return b.String()
	}
	b.WriteString("このノートでのこれまでのやり取り（古い順）です。新しい質問はこれを踏まえている場合があります。\n\n")
	for _, t := range turns {
		b.WriteString("[質問] " + t.Q + "\n[回答] " + t.A + "\n\n")
	}
	b.WriteString("---\n今回の質問（これに答えてください）:\n" + question)
	return b.String()
}

package claude

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// fakeClaude writes a script that prints the given stream-json lines and exits with `code`.
func fakeClaude(t *testing.T, lines []string, code int) string {
	t.Helper()
	dir := t.TempDir()
	path := filepath.Join(dir, "claude")
	var b strings.Builder
	b.WriteString("#!/bin/sh\ncat >/dev/null\n")
	for _, l := range lines {
		b.WriteString("printf '%s\\n' '" + l + "'\n")
	}
	b.WriteString("exit " + string(rune('0'+code)) + "\n")
	if err := os.WriteFile(path, []byte(b.String()), 0o755); err != nil {
		t.Fatal(err)
	}
	return path
}

const delta = `{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"%s"}}}`

func TestAskStreamsTextDeltas(t *testing.T) {
	exe := fakeClaude(t, []string{
		strings.Replace(delta, "%s", "こん", 1),
		`{"type":"system","subtype":"init"}`,
		strings.Replace(delta, "%s", "にちは", 1),
		`{"type":"result","is_error":false}`,
	}, 0)
	r, err := NewRunner(exe, "haiku")
	if err != nil {
		t.Fatal(err)
	}
	var got []string
	answer, err := r.Ask(context.Background(), "q", "sys", func(s string) { got = append(got, s) })
	if err != nil {
		t.Fatal(err)
	}
	if answer != "こんにちは" || strings.Join(got, "|") != "こん|にちは" {
		t.Fatalf("answer %q deltas %v", answer, got)
	}
}

func TestAskReportsResultErrors(t *testing.T) {
	exe := fakeClaude(t, []string{`{"type":"result","is_error":true,"result":"Not logged in"}`}, 0)
	r, _ := NewRunner(exe, "")
	if _, err := r.Ask(context.Background(), "q", "sys", func(string) {}); err == nil || !strings.Contains(err.Error(), "Not logged in") {
		t.Fatalf("got %v", err)
	}
}

func TestAskReportsNonZeroExit(t *testing.T) {
	exe := fakeClaude(t, nil, 3)
	r, _ := NewRunner(exe, "")
	if _, err := r.Ask(context.Background(), "q", "sys", func(string) {}); err == nil {
		t.Fatal("expected an error for exit status 3")
	}
}

func TestAskStopsWhenCancelled(t *testing.T) {
	dir := t.TempDir()
	exe := filepath.Join(dir, "claude")
	_ = os.WriteFile(exe, []byte("#!/bin/sh\nsleep 30\n"), 0o755)
	r, _ := NewRunner(exe, "")
	ctx, cancel := context.WithTimeout(context.Background(), 200*time.Millisecond)
	defer cancel()
	start := time.Now()
	_, err := r.Ask(ctx, "q", "sys", func(string) {})
	if err == nil || time.Since(start) > 5*time.Second {
		t.Fatalf("cancel should stop the process quickly (err=%v, took %v)", err, time.Since(start))
	}
}

func TestSystemPromptCarriesTheLengthBudget(t *testing.T) {
	if p := SystemPrompt("ja", 300); !strings.Contains(p, "300字以内") {
		t.Errorf("ja prompt: %q", p)
	}
	if p := SystemPrompt("en", 300); !strings.Contains(p, "60 words") {
		t.Errorf("en prompt: %q", p)
	}
	if p := SystemPrompt("ja", 1); !strings.Contains(p, "60字以内") {
		t.Errorf("budget must have a floor: %q", p)
	}
}

func TestAskActiveTellsTheProcessStartingFromTheModelWorking(t *testing.T) {
	exe := fakeClaude(t, []string{
		`{"type":"system","subtype":"init"}`,
		`{"type":"system","subtype":"status"}`,
		`{"type":"stream_event","event":{"type":"message_start"}}`,
		`{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"hm"}}}`,
		strings.Replace(delta, "%s", "答え", 1),
		`{"type":"result","is_error":false}`,
	}, 0)
	r, _ := NewRunner(exe, "")
	var signs []bool
	if _, err := r.AskActive(context.Background(), "q", "sys", func(string) {}, func(model bool) { signs = append(signs, model) }); err != nil {
		t.Fatal(err)
	}
	want := []bool{false, false, true, true, true, true}
	if len(signs) != len(want) {
		t.Fatalf("got %v", signs)
	}
	for i := range want {
		if signs[i] != want[i] {
			t.Fatalf("sign %d: got %v, want %v (all: %v)", i, signs[i], want[i], signs)
		}
	}
}

func TestAskActiveIgnoresLinesThatAreNotEvents(t *testing.T) {
	exe := fakeClaude(t, []string{`not json at all`, `{"type":"system"}`, `{"type":"result","is_error":false}`}, 0)
	r, _ := NewRunner(exe, "")
	n := 0
	if _, err := r.AskActive(context.Background(), "q", "sys", func(string) {}, func(bool) { n++ }); err != nil {
		t.Fatal(err)
	}
	if n != 2 {
		t.Fatalf("a line that is not JSON is no sign of life: got %d signs", n)
	}
}

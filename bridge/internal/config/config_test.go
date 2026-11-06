package config

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// Each test gets its own config directory.
func isolate(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)
	t.Setenv("HOME", dir) // macOS/others fall back to HOME
	return dir
}

func TestLoadCreatesAConfigWithTokenAndOwnTilt(t *testing.T) {
	isolate(t)
	c, err := Load()
	if err != nil {
		t.Fatal(err)
	}
	if len(c.Token) < 30 || c.Port != 8765 || c.ClaudeExecutable != "claude" || c.TiltDegrees == nil {
		t.Fatalf("unexpected defaults: %+v", c)
	}
	file, _ := ConfigFile()
	info, err := os.Stat(file)
	if err != nil || info.Mode().Perm() != 0o600 {
		t.Fatalf("config must exist with mode 0600, got %v %v", info, err)
	}
	again, _ := Load()
	if again.Token != c.Token || *again.TiltDegrees != *c.TiltDegrees {
		t.Fatal("the token and the writing slope must be stable across runs")
	}
}

func TestLoadNamesTheFileWhenJSONIsBroken(t *testing.T) {
	isolate(t)
	file, _ := ConfigFile()
	_ = os.MkdirAll(filepath.Dir(file), 0o700)
	_ = os.WriteFile(file, []byte(`{"name": "x"  "port": 1}`), 0o600)
	if _, err := Load(); err == nil || !contains(err.Error(), file) {
		t.Fatalf("error should name %s, got %v", file, err)
	}
}

func contains(s, sub string) bool { return len(s) >= len(sub) && (s == sub || indexOf(s, sub) >= 0) }
func indexOf(s, sub string) int {
	for i := 0; i+len(sub) <= len(s); i++ {
		if s[i:i+len(sub)] == sub {
			return i
		}
	}
	return -1
}

func TestSaveRoundTripAndLineTiltDefault(t *testing.T) {
	isolate(t)
	c, _ := Load()
	if !c.LineTiltOn() {
		t.Fatal("line tilt defaults to on")
	}
	off := false
	c.LineTilt, c.ShowProgress = &off, true
	if err := Save(c); err != nil {
		t.Fatal(err)
	}
	got, _ := Load()
	if got.LineTiltOn() || !got.ShowProgress {
		t.Fatalf("settings not persisted: %+v", got)
	}
}

func TestTiltStaysWithinAGentleRange(t *testing.T) {
	for i := 0; i < 200; i++ {
		c := &Config{}
		c.ensureTilt()
		if d := *c.TiltDegrees; d < -2.5 || d > 2.5 {
			t.Fatalf("tilt %v out of range", d)
		}
	}
}

func TestAppendReportWritesJSONLines(t *testing.T) {
	isolate(t)
	if err := AppendReport("peer", map[string]any{"a": 1}); err != nil {
		t.Fatal(err)
	}
	file, _ := ReportFile()
	b, _ := os.ReadFile(file)
	if len(b) == 0 || b[len(b)-1] != '\n' {
		t.Fatalf("expected one JSON line, got %q", b)
	}
}

func TestHistoryDefaultsAndSwitches(t *testing.T) {
	for _, tc := range []struct {
		h    *History
		want int
	}{
		{nil, 8000}, {&History{}, 8000}, {&History{ContextChars: 500}, 500}, {&History{ContextChars: -1}, 0},
	} {
		if got := (&Config{History: tc.h}).HistoryChars(); got != tc.want {
			t.Errorf("%+v: got %d, want %d", tc.h, got, tc.want)
		}
	}
}

func TestUnsupportedCompactionIsRejectedByName(t *testing.T) {
	dir := isolate(t)
	file := filepath.Join(dir, "spellink-bridge", "config.json")
	if err := os.MkdirAll(filepath.Dir(file), 0o700); err != nil {
		t.Fatal(err)
	}
	for mode, ok := range map[string]bool{"none": true, "": true, "summarize": false} {
		body := `{"token":"x","history":{"compaction":"` + mode + `"}}`
		if err := os.WriteFile(file, []byte(body), 0o600); err != nil {
			t.Fatal(err)
		}
		_, err := Load()
		if ok && err != nil || !ok && err == nil {
			t.Errorf("compaction %q: err=%v", mode, err)
		}
		if !ok && !strings.Contains(err.Error(), "config.json") {
			t.Errorf("the error must name the file: %v", err)
		}
	}
}

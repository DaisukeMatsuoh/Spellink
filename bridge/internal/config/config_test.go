package config

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
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

func TestProviderTimeoutDefaultsAndOverrides(t *testing.T) {
	first, gap, total := (&Config{}).ProviderTimeouts()
	if first != 10*time.Second || gap != 10*time.Second || total != 30*time.Second {
		t.Fatalf("defaults: %v %v %v", first, gap, total)
	}
	first, gap, total = (&Config{Timeouts: &Timeouts{FirstOutputSec: 5, TotalSec: 60}}).ProviderTimeouts()
	if first != 5*time.Second || gap != 10*time.Second || total != 60*time.Second {
		t.Fatalf("partial override: %v %v %v", first, gap, total)
	}
}

func TestOutOfRangeSettingsAreRejectedByName(t *testing.T) {
	dir := isolate(t)
	file := filepath.Join(dir, "spellink-bridge", "config.json")
	if err := os.MkdirAll(filepath.Dir(file), 0o700); err != nil {
		t.Fatal(err)
	}
	for body, want := range map[string]string{
		`{"token":"x","port":70000}`:                  "port",
		`{"token":"x","port":-1}`:                     "port",
		`{"token":"x","timeouts":{"gapSec":-5}}`:      "gapSec",
		`{"token":"x","timeouts":{"totalSec":99999}}`: "totalSec",
	} {
		if err := os.WriteFile(file, []byte(body), 0o600); err != nil {
			t.Fatal(err)
		}
		_, err := Load()
		if err == nil || !strings.Contains(err.Error(), want) || !strings.Contains(err.Error(), "config.json") {
			t.Errorf("%s: want an error naming %q and the file, got %v", body, want, err)
		}
	}
}

func TestTheReportLogIsMovedAsideWhenItGrowsTooLarge(t *testing.T) {
	isolate(t)
	file, _ := ReportFile()
	if err := os.MkdirAll(filepath.Dir(file), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(file, make([]byte, maxReportBytes+1), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := AppendReport("peer", map[string]any{"k": "v"}); err != nil {
		t.Fatal(err)
	}
	if info, err := os.Stat(file + ".1"); err != nil || info.Size() <= maxReportBytes {
		t.Fatalf("the old log must be kept as .1: %v %v", info, err)
	}
	if info, err := os.Stat(file); err != nil || info.Size() > 1000 {
		t.Fatalf("a fresh log must start: %v %v", info, err)
	}
}

func TestBusyNoticeIsOnUnlessTurnedOff(t *testing.T) {
	off := false
	if !(&Config{}).BusyNoticeOn() || (&Config{BusyNotice: &off}).BusyNoticeOn() {
		t.Fatal("the busy notice defaults to on and can be turned off")
	}
}

func TestProviderDefaultsToClaudeCodeAndTheOlderFieldsStillWork(t *testing.T) {
	kind, model, exe := (&Config{}).ProviderSettings()
	if kind != "claude-code" || model != "" || exe != "claude" {
		t.Fatalf("defaults: %s %q %s", kind, model, exe)
	}
	kind, model, exe = (&Config{Model: "haiku", ClaudeExecutable: "/opt/claude"}).ProviderSettings()
	if kind != "claude-code" || model != "haiku" || exe != "/opt/claude" {
		t.Fatalf("the older top-level fields must still count: %s %q %s", kind, model, exe)
	}
	kind, model, exe = (&Config{Model: "haiku", Provider: &ProviderConfig{Model: "sonnet", Executable: "/x/claude"}}).ProviderSettings()
	if kind != "claude-code" || model != "sonnet" || exe != "/x/claude" {
		t.Fatalf("provider.* wins over the older fields: %s %q %s", kind, model, exe)
	}
}

func TestAnUnknownProviderIsRefusedByNameAndWithTheChoices(t *testing.T) {
	dir := isolate(t)
	file := filepath.Join(dir, "spellink-bridge", "config.json")
	if err := os.MkdirAll(filepath.Dir(file), 0o700); err != nil {
		t.Fatal(err)
	}
	for body, ok := range map[string]bool{
		`{"token":"x"}`: true,
		`{"token":"x","provider":{"kind":"claude-code"}}`: true,
		`{"token":"x","provider":{"kind":"gpt-9"}}`:       false,
	} {
		if err := os.WriteFile(file, []byte(body), 0o600); err != nil {
			t.Fatal(err)
		}
		_, err := Load()
		if ok && err != nil {
			t.Errorf("%s: %v", body, err)
		}
		if !ok && (err == nil || !strings.Contains(err.Error(), "gpt-9") || !strings.Contains(err.Error(), "claude-code") || !strings.Contains(err.Error(), "config.json")) {
			t.Errorf("%s: the error must name the file, the kind and the choices: %v", body, err)
		}
	}
}

func TestPairingDefaultsAndOverrides(t *testing.T) {
	c := &Config{}
	if c.PairingWindow() != 90*time.Second || c.PairingTTL() != 24*time.Hour || c.PluginPackage() != "" {
		t.Errorf("defaults: %v %v %q", c.PairingWindow(), c.PairingTTL(), c.PluginPackage())
	}
	c = &Config{Pairing: &Pairing{}}
	if c.PairingWindow() != 90*time.Second || c.PairingTTL() != 24*time.Hour {
		t.Error("zero fields mean the defaults")
	}
	c = &Config{Pairing: &Pairing{WindowSec: 30, TTLHours: 2, PluginPackage: "/x/Spellink.snplg"}}
	if c.PairingWindow() != 30*time.Second || c.PairingTTL() != 2*time.Hour || c.PluginPackage() != "/x/Spellink.snplg" {
		t.Errorf("overrides: %v %v %q", c.PairingWindow(), c.PairingTTL(), c.PluginPackage())
	}
}

func TestOutOfRangePairingSettingsAreRejectedByName(t *testing.T) {
	for name, p := range map[string]Pairing{
		"pairing.windowSec": {WindowSec: 9}, "pairing.windowSec ": {WindowSec: 601}, "pairing.ttlHours": {TTLHours: -1}, "pairing.ttlHours ": {TTLHours: 24*30 + 1},
	} {
		c := &Config{Pairing: &p}
		if err := c.validate("config.json"); err == nil || !strings.Contains(err.Error(), strings.TrimSpace(name)) {
			t.Errorf("%s %+v: %v", name, p, err)
		}
	}
	ok := &Config{Pairing: &Pairing{WindowSec: 10, TTLHours: 24 * 30}}
	if err := ok.validate("config.json"); err != nil {
		t.Errorf("the limits themselves are allowed: %v", err)
	}
}

func TestAdminTokenIsCreatedOnceAndPrivate(t *testing.T) {
	isolate(t)
	a, err := AdminToken()
	if err != nil || len(a) < 32 {
		t.Fatalf("token %q err %v", a, err)
	}
	b, _ := AdminToken()
	if a != b {
		t.Error("the token must stay the same once created")
	}
	file, _ := AdminTokenFile()
	if st, err := os.Stat(file); err != nil || st.Mode().Perm() != 0o600 {
		t.Errorf("mode %v err %v", st.Mode(), err)
	}
}

func TestAShortOrBrokenAdminTokenIsReplacedNotTrusted(t *testing.T) {
	isolate(t)
	file, _ := AdminTokenFile()
	os.MkdirAll(filepath.Dir(file), 0o700)
	os.WriteFile(file, []byte("short\n"), 0o644)
	tok, err := AdminToken()
	if err != nil || len(tok) < 32 || tok == "short" {
		t.Fatalf("token %q err %v", tok, err)
	}
	if st, _ := os.Stat(file); st.Mode().Perm() != 0o600 {
		t.Errorf("a replaced token file must become private, is %v", st.Mode())
	}
	// a file that cannot be read at all is an error, not a reason to create a new token
	os.Remove(file)
	os.MkdirAll(file, 0o700) // a directory where the file should be
	if _, err := AdminToken(); err == nil {
		t.Error("an unreadable token file must fail")
	}
}

func TestDevicesFileLivesInTheConfigDirectory(t *testing.T) {
	dir := isolate(t)
	f, err := DevicesFile()
	if err != nil || filepath.Dir(f) != filepath.Join(dir, "spellink-bridge") || filepath.Base(f) != "devices.json" {
		t.Errorf("%q %v", f, err)
	}
}

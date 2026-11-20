package main

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io/fs"
	"net"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"syscall"
	"testing"
	"time"

	"github.com/coder/websocket"

	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/probe"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/securechan"
)

// changeASetting does what the plugin's settings screen does: over the encrypted channel it sends
// setSettings, which makes the bridge save config.json. Without a save, an update test would never
// touch the code that could lose a user's settings.
func changeASetting(t *testing.T, port int, token string) {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	c, _, err := websocket.Dial(ctx, fmt.Sprintf("ws://127.0.0.1:%d/v1", port), nil)
	if err != nil {
		t.Fatal(err)
	}
	defer c.CloseNow()
	ch, err := securechan.Dial(ctx, c, securechan.PSK(token))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := ch.Read(ctx); err != nil { // welcome
		t.Fatal(err)
	}
	msg, _ := json.Marshal(map[string]any{"type": "setSettings", "settings": map[string]any{
		"showProgress": true, "lineTilt": true, "thinkingAnimation": false,
		"fullWidthSpacing": 120, "halfWidthSpacing": 130, "busyNotice": false,
	}})
	if err := ch.Write(ctx, msg); err != nil {
		t.Fatal(err)
	}
	if _, err := ch.Read(ctx); err != nil { // the settings reply: the save has happened
		t.Fatal(err)
	}
}

// Updating the bridge means replacing its file and starting it again. Nothing the user has —
// the token (every paired device depends on it), the settings, the devices, the conversations,
// the reports — may be lost or changed by that, including settings this version does not know.
func TestOverwritingTheBridgeKeepsEverythingTheUserHas(t *testing.T) {
	if testing.Short() {
		t.Skip("builds the bridge twice")
	}
	tmp := t.TempDir()
	exe := filepath.Join(tmp, "bin", "spellink-bridge")
	cfgDir := filepath.Join(tmp, "cfg", "spellink-bridge")
	if err := os.MkdirAll(filepath.Join(cfgDir, "history"), 0o700); err != nil {
		t.Fatal(err)
	}
	build := func(version string) {
		t.Helper()
		cmd := exec.Command("go", "build", "-o", exe, "-ldflags", "-X main.version="+version, ".")
		if out, err := cmd.CombinedOutput(); err != nil {
			t.Fatalf("build %s: %v\n%s", version, err, out)
		}
	}
	build("1.0.0")
	if out, err := exec.Command(exe, "testfont", filepath.Join(cfgDir, "glyphs.bin")).CombinedOutput(); err != nil {
		t.Fatalf("%v %s", err, out)
	}
	port := freePort(t)
	config := map[string]any{
		"name": "upgrade", "port": port, "token": "the-token-every-device-depends-on",
		"claudeExecutable": "/bin/false", "model": "haiku", "tiltDegrees": 0.4,
		"extraUrls":  []string{"wss://bridge.example.com/v1"},
		"timeouts":   map[string]any{"firstOutputSec": 12},
		"history":    map[string]any{"contextChars": 6000},
		"busyNotice": false,
		// written by a newer version: this one must keep it untouched
		"someFutureSetting": map[string]any{"enabled": true, "level": 3},
	}
	writeJSON(t, filepath.Join(cfgDir, "config.json"), config)
	mustWrite(t, filepath.Join(cfgDir, "history", "0123456789abcdef.jsonl"), `{"at":"2026-11-01T00:00:00Z","q":"1+1は？","a":"2です。"}`+"\n")
	mustWrite(t, filepath.Join(cfgDir, "reports.jsonl"), `{"at":"2026-11-01T00:00:00Z","peer":"x","data":{}}`+"\n")

	env := append(os.Environ(), "HOME="+tmp, "XDG_CONFIG_HOME="+filepath.Join(tmp, "cfg"))
	run := func(wantVersion string) {
		t.Helper()
		cmd := exec.Command(exe, "serve")
		cmd.Env = env
		out, _ := os.Create(filepath.Join(tmp, "serve-"+wantVersion+".log"))
		defer out.Close()
		cmd.Stdout, cmd.Stderr = out, out
		if err := cmd.Start(); err != nil {
			t.Fatal(err)
		}
		defer func() {
			_ = cmd.Process.Signal(syscall.SIGTERM)
			done := make(chan error, 1)
			go func() { done <- cmd.Wait() }()
			select {
			case <-done:
			case <-time.After(10 * time.Second):
				_ = cmd.Process.Kill()
				t.Error("the bridge did not stop on SIGTERM")
			}
		}()
		url := fmt.Sprintf("http://127.0.0.1:%d/health", port)
		for i := 0; ; i++ {
			if r, err := http.Get(url); err == nil {
				r.Body.Close()
				break
			}
			if i > 100 {
				log, _ := os.ReadFile(out.Name())
				t.Fatalf("version %s did not start:\n%s", wantVersion, log)
			}
			time.Sleep(100 * time.Millisecond)
		}
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		// the same token still opens the encrypted channel: every paired device keeps working
		r, err := probe.Probe(ctx, fmt.Sprintf("ws://127.0.0.1:%d/v1", port), "the-token-every-device-depends-on")
		if err != nil {
			t.Fatalf("version %s: the old token no longer works: %v", wantVersion, err)
		}
		if r.Version != wantVersion || r.Bridge != "upgrade" {
			t.Fatalf("got %+v, want version %s", r, wantVersion)
		}
	}

	run("1.0.0")
	before := snapshot(t, cfgDir)

	build("1.1.0") // overwrite the same file, as an update does
	run("1.1.0")
	after := snapshot(t, cfgDir)

	// …and then the user changes a setting on the device, which makes the new version SAVE
	// config.json. That is where a key it does not know could be lost.
	saveRun := exec.Command(exe, "serve")
	saveRun.Env = env
	if err := saveRun.Start(); err != nil {
		t.Fatal(err)
	}
	defer func() { _ = saveRun.Process.Kill(); _, _ = saveRun.Process.Wait() }()
	for i := 0; ; i++ {
		if r, err := http.Get(fmt.Sprintf("http://127.0.0.1:%d/health", port)); err == nil {
			r.Body.Close()
			break
		}
		if i > 100 {
			t.Fatal("the bridge did not start for the settings change")
		}
		time.Sleep(100 * time.Millisecond)
	}
	changeASetting(t, port, "the-token-every-device-depends-on")
	config["showProgress"], config["lineTilt"] = true, true
	config["fullWidthSpacing"], config["halfWidthSpacing"] = 120, 130
	config["busyNotice"] = false

	for path, sum := range before {
		got, ok := after[path]
		switch {
		case !ok:
			t.Errorf("%s was deleted by the update", path)
		case got != sum && path != "config.json":
			t.Errorf("%s was changed by the update", path)
		}
	}
	var cfg map[string]any
	raw, _ := os.ReadFile(filepath.Join(cfgDir, "config.json"))
	if err := json.Unmarshal(raw, &cfg); err != nil {
		t.Fatalf("config.json is no longer JSON: %v", err)
	}
	for k, want := range config {
		if k == "port" {
			want = float64(port)
		}
		if fmt.Sprint(cfg[k]) != fmt.Sprint(normalize(want)) {
			t.Errorf("config %q: was %v, is %v", k, normalize(want), cfg[k])
		}
	}
	if info, _ := os.Stat(filepath.Join(cfgDir, "config.json")); info.Mode().Perm() != 0o600 {
		t.Errorf("config.json must stay readable by its owner only: %v", info.Mode())
	}
}

func normalize(v any) any {
	b, _ := json.Marshal(v)
	var out any
	_ = json.Unmarshal(b, &out)
	return out
}

func snapshot(t *testing.T, dir string) map[string]string {
	t.Helper()
	out := map[string]string{}
	_ = filepath.WalkDir(dir, func(p string, d fs.DirEntry, err error) error {
		if err != nil || d.IsDir() {
			return err
		}
		b, err := os.ReadFile(p)
		if err != nil {
			return err
		}
		h := sha256.Sum256(b)
		rel, _ := filepath.Rel(dir, p)
		out[rel] = hex.EncodeToString(h[:])
		return nil
	})
	return out
}

func freePort(t *testing.T) int {
	t.Helper()
	l, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer l.Close()
	return l.Addr().(*net.TCPAddr).Port
}

func writeJSON(t *testing.T, path string, v any) {
	t.Helper()
	b, _ := json.MarshalIndent(v, "", "  ")
	mustWrite(t, path, string(b))
}

func mustWrite(t *testing.T, path, s string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(s), 0o600); err != nil {
		t.Fatal(err)
	}
}

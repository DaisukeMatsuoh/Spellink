package server

import (
	"context"
	"encoding/json"
	"flag"
	"os"
	"sort"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
)

var update = flag.Bool("update", false, "rewrite internal/server/testdata/frames.golden.json")

const goldenPath = "testdata/frames.golden.json"

// shape names a frame by its type, its stage or code, and the fields it carries: the plugin's
// validator (plugin/src/frames.ts) depends on exactly these.
func shape(m msg) string {
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	kind := m["type"].(string)
	if s, ok := m["stage"].(string); ok {
		kind += ":" + s
	}
	if c, ok := m["code"].(string); ok {
		kind += ":" + c
	}
	if p, ok := m["phase"].(string); ok && m["stage"] == "alive" {
		kind += ":" + p
	}
	return kind + " {" + strings.Join(keys, ",") + "}"
}

// readUntilQuiet collects every frame that arrives until nothing comes for `quiet`.
func readUntilQuiet(c *websocket.Conn, quiet time.Duration) []msg {
	var out []msg
	for {
		ctx, cancel := context.WithTimeout(context.Background(), quiet)
		_, b, err := c.Read(ctx)
		cancel()
		if err != nil {
			return out
		}
		var m msg
		_ = json.Unmarshal(b, &m)
		out = append(out, m)
	}
}

// The frames the bridge really sends, from a real session of every kind, are recorded in
// testdata/frames.golden.json; the plugin's tests (plugin/__tests__/contract.test.ts) check that
// its validator accepts every one of them. If this fails after a deliberate change to a frame:
// run `go test ./internal/server -run Contract -update`, then the plugin's tests.
func TestContractTheFramesTheBridgeSendsAreTheOnesThePluginExpects(t *testing.T) {
	url := startTestWith(t, func(ctx context.Context, q, _ string, onText func(string)) (string, error) {
		if strings.Contains(q, "cancel-me") { // says nothing at all until it is cancelled
			<-ctx.Done()
			return "", ctx.Err()
		}
		onText("あああ\n")
		time.Sleep(80 * time.Millisecond) // long enough for keep-alives while writing
		onText("あ")
		return "あああ\nあ", nil
	}, func(d *Deps) { d.KeepaliveEvery = 20 * time.Millisecond })

	var frames []msg
	// a full answer: welcome, status, alive, line, done
	c, _ := dial(t, url)
	send(t, c, ask("g1", "/n/a.note", "normal"))
	frames = append(frames, readUntilQuiet(c, 300*time.Millisecond)...)
	// a cancelled answer that never started: keep-alives while starting, then the ending error
	c2, _ := dial(t, url)
	send(t, c2, ask("g2", "/n/b.note", "cancel-me"))
	time.Sleep(100 * time.Millisecond)
	send(t, c2, map[string]any{"type": "cancel", "id": "g2"})
	frames = append(frames, readUntilQuiet(c2, 300*time.Millisecond)...)
	// an answer that is not kept, settings, the memory
	c3, _ := dial(t, url)
	send(t, c3, map[string]any{"type": "resume", "id": "nobody", "next": 0})
	send(t, c3, map[string]any{"type": "setSettings", "settings": map[string]any{"showProgress": true}})
	send(t, c3, map[string]any{"type": "clearHistory"})
	frames = append(frames, readUntilQuiet(c3, 300*time.Millisecond)...)
	// the welcome of a fresh connection
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	w, _, err := websocket.Dial(ctx, url, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer w.CloseNow()
	b, _ := json.Marshal(hello)
	_ = w.Write(ctx, websocket.MessageText, b)
	if _, wb, err := w.Read(ctx); err == nil {
		var m msg
		_ = json.Unmarshal(wb, &m)
		frames = append(frames, m)
	}

	// one example of each shape
	seen := map[string]msg{}
	for _, f := range frames {
		if _, ok := f["type"].(string); ok {
			if _, dup := seen[shape(f)]; !dup {
				seen[shape(f)] = f
			}
		}
	}
	shapes := make([]string, 0, len(seen))
	for k := range seen {
		shapes = append(shapes, k)
	}
	sort.Strings(shapes)
	for _, want := range []string{"welcome", "status:thinking", "status:writing", "status:alive:starting", "line", "done", "error:cancelled", "error:unknown_ask", "settings", "historyCleared"} {
		found := false
		for _, s := range shapes {
			if strings.HasPrefix(s, want+" ") || strings.HasPrefix(s, want+":") {
				found = true
			}
		}
		if !found {
			t.Fatalf("the session produced no %q frame; got %v", want, shapes)
		}
	}
	examples := make([]msg, 0, len(shapes))
	for _, s := range shapes {
		examples = append(examples, seen[s])
	}
	out, _ := json.MarshalIndent(examples, "", "  ")

	if *update {
		if err := os.WriteFile(goldenPath, append(out, '\n'), 0o644); err != nil {
			t.Fatal(err)
		}
		return
	}
	raw, err := os.ReadFile(goldenPath)
	if err != nil {
		t.Fatalf("no golden file (run with -update): %v", err)
	}
	var golden []msg
	_ = json.Unmarshal(raw, &golden)
	got := map[string]bool{}
	for _, g := range golden {
		got[shape(g)] = true
	}
	for _, s := range shapes {
		if !got[s] {
			t.Errorf("a frame the golden file does not have: %s (run with -update, then the plugin tests)", s)
		}
	}
	for s := range got {
		if _, ok := seen[s]; !ok {
			t.Errorf("the golden file has a frame the bridge no longer sends: %s", s)
		}
	}
}

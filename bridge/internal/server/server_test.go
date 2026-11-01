package server

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"log"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"

	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/ink"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/protocol"
)

var layout = ink.Layout{X: 10, Y: 10, Width: 400, PageHeight: 1000, BottomMargin: 20, CharSize: 40}

func startTest(t *testing.T, ask AskFunc) string {
	t.Helper()
	font := &ink.Font{Em: 100, Glyphs: map[rune]ink.Glyph{'あ': {Advance: 100, Strokes: [][]int16{{50, 0, 50, 100}}}}}
	srv := httptest.NewServer(Handler(Deps{
		Name: "test", Version: "0", Token: "secret", Font: font, Ask: ask,
		Report:   func(string, any) {},
		Settings: func() protocol.Settings { return protocol.Settings{} }, SetSettings: func(protocol.Settings) error { return nil }, Log: log.New(io.Discard, "", 0),
	}))
	t.Cleanup(srv.Close)
	return "ws" + strings.TrimPrefix(srv.URL, "http") + "/v1"
}

type msg map[string]any

// converse sends frames in order and collects replies until `until` matches or the socket closes.
func converse(t *testing.T, url string, frames []any, until func(msg) bool) []msg {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	c, _, err := websocket.Dial(ctx, url, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer c.CloseNow()
	for _, f := range frames {
		b, _ := json.Marshal(f)
		if err := c.Write(ctx, websocket.MessageText, b); err != nil {
			t.Fatal(err)
		}
	}
	var got []msg
	for {
		_, b, err := c.Read(ctx)
		if err != nil {
			return got
		}
		var m msg
		_ = json.Unmarshal(b, &m)
		got = append(got, m)
		if until(m) {
			return got
		}
	}
}

func types(ms []msg) string {
	var s []string
	for _, m := range ms {
		s = append(s, m["type"].(string))
	}
	return strings.Join(s, ",")
}

var hello = map[string]any{"type": "hello", "token": "secret", "client": "t"}

func TestRejectsWrongToken(t *testing.T) {
	url := startTest(t, nil)
	got := converse(t, url, []any{map[string]any{"type": "hello", "token": "nope"}}, func(msg) bool { return false })
	if types(got) != "error" || got[0]["code"] != "unauthorized" {
		t.Fatalf("got %v", got)
	}
}

func TestStreamsLinesAndFinishes(t *testing.T) {
	var prompt string
	url := startTest(t, func(_ context.Context, q, sys string, onText func(string)) (string, error) {
		prompt = sys
		onText("あああ")
		onText("\nあ")
		return "あああ\nあ", nil
	})
	got := converse(t, url, []any{hello,
		map[string]any{"type": "ask", "id": "q1", "question": "test?", "lang": "ja", "layout": layout},
	}, func(m msg) bool { return m["type"] == "done" || m["type"] == "error" })
	if types(got) != "welcome,status,status,line,line,done" {
		t.Fatalf("got %s", types(got))
	}
	if got[3]["text"] != "あああ" || len(got[3]["strokes"].([]any)) != 3 || got[3]["page"] != 0.0 {
		t.Fatalf("first line: %v", got[3])
	}
	if got[5]["pages"] != 1.0 {
		t.Fatalf("done: %v", got[5])
	}
	if !strings.Contains(prompt, "字以内") {
		t.Fatalf("system prompt lacks a length budget: %q", prompt)
	}
}

func TestReportsClaudeFailure(t *testing.T) {
	url := startTest(t, func(context.Context, string, string, func(string)) (string, error) {
		return "", errors.New("not logged in")
	})
	got := converse(t, url, []any{hello,
		map[string]any{"type": "ask", "id": "q2", "question": "x", "lang": "en", "layout": layout},
	}, func(m msg) bool { return m["type"] == "error" })
	if last := got[len(got)-1]; last["code"] != "claude_failed" {
		t.Fatalf("got %v", last)
	}
}

func TestCancelStopsTheModel(t *testing.T) {
	stopped := make(chan struct{})
	url := startTest(t, func(ctx context.Context, _, _ string, _ func(string)) (string, error) {
		<-ctx.Done()
		close(stopped)
		return "", ctx.Err()
	})
	got := converse(t, url, []any{hello,
		map[string]any{"type": "ask", "id": "q3", "question": "x", "lang": "ja", "layout": layout},
		map[string]any{"type": "cancel", "id": "q3"},
	}, func(m msg) bool { return m["type"] == "error" })
	<-stopped
	if last := got[len(got)-1]; last["code"] != "cancelled" {
		t.Fatalf("got %v", last)
	}
}

func TestValidLayout(t *testing.T) {
	ok := layout
	if !ValidLayout(&ok) {
		t.Fatal("valid layout rejected")
	}
	for name, mut := range map[string]func(*ink.Layout){
		"charSize": func(l *ink.Layout) { l.CharSize = 0 },
		"y":        func(l *ink.Layout) { l.Y = 5000 },
		"maxPages": func(l *ink.Layout) { l.MaxPages = 99 },
		"top":      func(l *ink.Layout) { l.ContinueTop = -1 },
	} {
		l := layout
		mut(&l)
		if ValidLayout(&l) {
			t.Errorf("%s: invalid layout accepted", name)
		}
	}
	if ValidLayout(nil) {
		t.Error("nil layout accepted")
	}
}

func TestProxiedConnectionsMustBeEncrypted(t *testing.T) {
	url := startTest(t, nil)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	c, _, err := websocket.Dial(ctx, url, &websocket.DialOptions{
		HTTPHeader: map[string][]string{"Cf-Connecting-Ip": {"203.0.113.9"}},
	})
	if err != nil {
		t.Fatal(err)
	}
	defer c.CloseNow()
	b, _ := json.Marshal(hello)
	_ = c.Write(ctx, websocket.MessageText, b)
	_, reply, err := c.Read(ctx)
	if err != nil || !strings.Contains(string(reply), "unauthorized") {
		t.Fatalf("plaintext hello through a proxy should be refused, got %q %v", reply, err)
	}
}

func TestSettingsRoundTrip(t *testing.T) {
	stored := protocol.Settings{}
	font := &ink.Font{Em: 100, Glyphs: map[rune]ink.Glyph{}}
	srv := httptest.NewServer(Handler(Deps{
		Name: "test", Version: "0", Token: "secret", Font: font, Log: log.New(io.Discard, "", 0),
		Report:      func(string, any) {},
		Settings:    func() protocol.Settings { return stored },
		SetSettings: func(s protocol.Settings) error { stored = s; return nil },
	}))
	defer srv.Close()
	url := "ws" + strings.TrimPrefix(srv.URL, "http") + "/v1"
	got := converse(t, url, []any{hello,
		map[string]any{"type": "setSettings", "settings": map[string]any{"showProgress": true}},
	}, func(m msg) bool { return m["type"] == "settings" })
	if types(got) != "welcome,settings" {
		t.Fatalf("got %s", types(got))
	}
	if got[0]["settings"].(map[string]any)["showProgress"] != false {
		t.Fatalf("welcome should carry the old settings: %v", got[0])
	}
	if got[1]["settings"].(map[string]any)["showProgress"] != true || !stored.ShowProgress {
		t.Fatalf("setting not applied: %v", got[1])
	}
}

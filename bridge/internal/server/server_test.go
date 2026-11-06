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

	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/history"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/ink"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/protocol"
)

var layout = ink.Layout{X: 10, Y: 10, Width: 400, PageHeight: 1000, BottomMargin: 20, CharSize: 40}

func startTest(t *testing.T, ask AskFunc) string {
	t.Helper()
	return startTestWith(t, ask, func(*Deps) {})
}

func startTestWith(t *testing.T, ask AskFunc, tweak func(*Deps)) string {
	t.Helper()
	font := &ink.Font{Em: 100, Glyphs: map[rune]ink.Glyph{'あ': {Advance: 100, Strokes: [][]int16{{50, 0, 50, 100}}}}}
	d := Deps{
		Name: "test", Version: "0", Token: "secret", Font: font, Ask: ask,
		Report:   func(string, any) {},
		Settings: func() protocol.Settings { return protocol.Settings{} }, SetSettings: func(protocol.Settings) error { return nil }, Log: log.New(io.Discard, "", 0),
	}
	tweak(&d)
	srv := httptest.NewServer(Handler(d))
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

func finished(m msg) bool { return m["type"] == "done" || m["type"] == "error" }

func ask(id, note, q string) map[string]any {
	return map[string]any{"type": "ask", "id": id, "note": note, "question": q, "lang": "ja", "layout": layout}
}

func TestFollowUpInTheSameNoteSeesTheEarlierConversation(t *testing.T) {
	var asked []string
	url := startTestWith(t, func(_ context.Context, q, _ string, onText func(string)) (string, error) {
		asked = append(asked, q)
		onText("あ")
		return "答え:" + q, nil
	}, func(d *Deps) {
		d.History, d.HistoryChars = &history.Store{Dir: t.TempDir()}, 1000
	})
	// separate connections: the memory belongs to the note, not to the socket
	converse(t, url, []any{hello, ask("1", "/n/a.note", "1+1は？")}, finished)
	converse(t, url, []any{hello, ask("2", "/n/a.note", "それを3倍して")}, finished)
	converse(t, url, []any{hello, ask("3", "/n/other.note", "別の話")}, finished)

	if asked[0] != "1+1は？" {
		t.Fatalf("a first question is sent as is: %q", asked[0])
	}
	for _, want := range []string{"1+1は？", "答え:1+1は？", "それを3倍して"} {
		if !strings.Contains(asked[1], want) {
			t.Fatalf("follow-up lacks %q: %q", want, asked[1])
		}
	}
	if asked[2] != "別の話" {
		t.Fatalf("another note must not see it: %q", asked[2])
	}
}

func TestFailedOrCancelledQuestionsAreNotRemembered(t *testing.T) {
	store := &history.Store{Dir: t.TempDir()}
	url := startTestWith(t, func(context.Context, string, string, func(string)) (string, error) {
		return "", errors.New("boom")
	}, func(d *Deps) { d.History, d.HistoryChars = store, 1000 })
	converse(t, url, []any{hello, ask("1", "n", "q")}, finished)
	if got, _ := store.Recent("n", 1000); len(got) != 0 {
		t.Fatalf("remembered a failed question: %+v", got)
	}
}

func TestNoteWithoutKeyOrWithMemoryOffIsStateless(t *testing.T) {
	var asked []string
	record := func(_ context.Context, q, _ string, onText func(string)) (string, error) {
		asked = append(asked, q)
		onText("あ")
		return "a", nil
	}
	store := &history.Store{Dir: t.TempDir()}
	on := startTestWith(t, record, func(d *Deps) { d.History, d.HistoryChars = store, 1000 })
	converse(t, on, []any{hello, ask("1", "", "q1")}, finished)
	converse(t, on, []any{hello, ask("2", "", "q2")}, finished)
	off := startTestWith(t, record, func(d *Deps) { d.History, d.HistoryChars = store, 0 })
	converse(t, off, []any{hello, ask("3", "n", "q3")}, finished)
	converse(t, off, []any{hello, ask("4", "n", "q4")}, finished)
	if asked[1] != "q2" || asked[3] != "q4" {
		t.Fatalf("expected stateless questions, got %q", asked)
	}
}

func TestClearHistoryForgetsAllNotes(t *testing.T) {
	store := &history.Store{Dir: t.TempDir()}
	_ = store.Append("a", history.Turn{Q: "q", A: "a"})
	_ = store.Append("b", history.Turn{Q: "q", A: "a"})
	url := startTestWith(t, nil, func(d *Deps) { d.History, d.HistoryChars = store, 1000 })
	got := converse(t, url, []any{hello, map[string]any{"type": "clearHistory"}},
		func(m msg) bool { return m["type"] == "historyCleared" })
	if got[1]["notes"] != 2.0 {
		t.Fatalf("got %v", got)
	}
	if r, _ := store.Recent("a", 1000); len(r) != 0 {
		t.Fatalf("not cleared: %+v", r)
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
		"limit":    func(l *ink.Layout) { l.StartLimit = 99999 },
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

func TestSettingsAreClampedAndPersistedTogether(t *testing.T) {
	stored := protocol.Settings{FullWidthSpacing: 100, HalfWidthSpacing: 110, ThinkingAnimation: true}
	font := &ink.Font{Em: 100, Glyphs: map[rune]ink.Glyph{}}
	srv := httptest.NewServer(Handler(Deps{
		Name: "t", Version: "0", Token: "secret", Font: font, Log: log.New(io.Discard, "", 0),
		Report:      func(string, any) {},
		Settings:    func() protocol.Settings { return stored },
		SetSettings: func(s protocol.Settings) error { stored = s; return nil },
	}))
	defer srv.Close()
	url := "ws" + strings.TrimPrefix(srv.URL, "http") + "/v1"
	got := converse(t, url, []any{hello, map[string]any{"type": "setSettings", "settings": map[string]any{
		"thinkingAnimation": false, "fullWidthSpacing": 120, "halfWidthSpacing": 130,
	}}}, func(m msg) bool { return m["type"] == "settings" })
	s := got[len(got)-1]["settings"].(map[string]any)
	if s["thinkingAnimation"] != false || s["fullWidthSpacing"] != 120.0 || s["halfWidthSpacing"] != 130.0 {
		t.Fatalf("got %v", s)
	}
}

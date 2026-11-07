package server

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
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

// dial opens a connection, says hello and returns it with a reader of the next frame.
func dial(t *testing.T, url string) (*websocket.Conn, func() msg) {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	t.Cleanup(cancel)
	c, _, err := websocket.Dial(ctx, url, nil)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { c.CloseNow() })
	write := func(v any) {
		b, _ := json.Marshal(v)
		if err := c.Write(ctx, websocket.MessageText, b); err != nil {
			t.Fatal(err)
		}
	}
	write(hello)
	_, _, _ = c.Read(ctx) // welcome
	read := func() msg {
		_, b, err := c.Read(ctx)
		if err != nil {
			t.Fatalf("read: %v", err)
		}
		var m msg
		_ = json.Unmarshal(b, &m)
		return m
	}
	return c, read
}

func send(t *testing.T, c *websocket.Conn, v any) {
	t.Helper()
	b, _ := json.Marshal(v)
	if err := c.Write(context.Background(), websocket.MessageText, b); err != nil {
		t.Fatal(err)
	}
}

func TestDroppedConnectionCanFetchTheRestOfTheAnswer(t *testing.T) {
	release := make(chan struct{})
	url := startTest(t, func(_ context.Context, _, _ string, onText func(string)) (string, error) {
		onText("あああ\n") // one whole line before the connection drops
		<-release
		onText("あ\nあ")
		return "あああ\nあ\nあ", nil
	})
	c, read := dial(t, url)
	send(t, c, ask("a1", "", "q"))
	var first msg
	for first = read(); first["type"] != "line"; first = read() {
	}
	if first["seq"] != 0.0 {
		t.Fatalf("first frame must be number 0: %v", first)
	}
	c.CloseNow() // the tunnel drops while Claude is still answering
	close(release)

	c2, read2 := dial(t, url)
	send(t, c2, map[string]any{"type": "resume", "id": "a1", "next": 1})
	var seqs []float64
	for m := read2(); ; m = read2() {
		if m["type"] == "status" {
			continue
		}
		seqs = append(seqs, m["seq"].(float64))
		if m["type"] == "done" {
			break
		}
		if m["type"] == "error" {
			t.Fatalf("unexpected error: %v", m)
		}
	}
	if len(seqs) < 2 || seqs[0] != 1 {
		t.Fatalf("must continue at frame 1 without repeating frame 0, got %v", seqs)
	}
	for i := 1; i < len(seqs); i++ {
		if seqs[i] != seqs[i-1]+1 {
			t.Fatalf("frames must be consecutive: %v", seqs)
		}
	}
}

func TestResumeOfAnUnknownAnswerSaysSo(t *testing.T) {
	c, read := dial(t, startTest(t, nil))
	send(t, c, map[string]any{"type": "resume", "id": "never", "next": 0})
	if m := read(); m["type"] != "error" || m["code"] != "unknown_ask" {
		t.Fatalf("got %v", m)
	}
}

func TestAnswerNobodyComesBackForIsCancelled(t *testing.T) {
	stopped := make(chan struct{})
	url := startTestWith(t, func(ctx context.Context, _, _ string, onText func(string)) (string, error) {
		onText("あ")
		<-ctx.Done()
		close(stopped)
		return "", ctx.Err()
	}, func(d *Deps) { d.OrphanGrace = 50 * time.Millisecond })
	c, read := dial(t, url)
	send(t, c, ask("o1", "", "q"))
	for m := read(); m["type"] != "status" || m["stage"] != "writing"; m = read() {
	}
	c.CloseNow()
	select {
	case <-stopped:
	case <-time.After(3 * time.Second):
		t.Fatal("an answer with no plugin attached must stop spending Claude usage")
	}
}

func TestResumingBeforeTheGraceEndsKeepsTheAnswerAlive(t *testing.T) {
	release := make(chan struct{})
	url := startTestWith(t, func(ctx context.Context, _, _ string, onText func(string)) (string, error) {
		onText("あ")
		select {
		case <-release:
			return "あ", nil
		case <-ctx.Done():
			return "", ctx.Err()
		}
	}, func(d *Deps) { d.OrphanGrace = 300 * time.Millisecond })
	c, read := dial(t, url)
	send(t, c, ask("k1", "", "q"))
	for m := read(); m["type"] != "status" || m["stage"] != "writing"; m = read() {
	}
	c.CloseNow()
	c2, read2 := dial(t, url)
	send(t, c2, map[string]any{"type": "resume", "id": "k1", "next": 0})
	time.Sleep(600 * time.Millisecond) // well past the grace
	close(release)
	for m := read2(); ; m = read2() {
		if m["type"] == "done" {
			return
		}
		if m["type"] == "error" {
			t.Fatalf("the resumed answer was cancelled: %v", m)
		}
	}
}

func timeoutDeps(first, gap, total time.Duration) func(*Deps) {
	return func(d *Deps) { d.FirstOutput, d.Gap, d.Total = first, gap, total }
}

// lastFrame reads frames until the answer ends and returns the terminating one.
func lastFrame(read func() msg) msg {
	for m := read(); ; m = read() {
		if m["type"] == "done" || m["type"] == "error" {
			return m
		}
	}
}

func TestAnAIThatNeverStartsIsStopped(t *testing.T) {
	url := startTestWith(t, func(ctx context.Context, _, _ string, _ func(string)) (string, error) {
		<-ctx.Done()
		return "", ctx.Err()
	}, timeoutDeps(60*time.Millisecond, time.Second, 5*time.Second))
	c, read := dial(t, url)
	send(t, c, ask("t1", "", "q"))
	m := lastFrame(read)
	if m["type"] != "error" || m["code"] != "provider_timeout" || !strings.Contains(m["message"].(string), "no sign") {
		t.Fatalf("got %v", m)
	}
	if _, ok := m["seq"]; !ok {
		t.Fatalf("the error ends the answer, so it carries a sequence number: %v", m)
	}
}

func TestAnAIThatStopsMidAnswerIsStopped(t *testing.T) {
	url := startTestWith(t, func(ctx context.Context, _, _ string, onText func(string)) (string, error) {
		onText("あ")
		<-ctx.Done()
		return "", ctx.Err()
	}, timeoutDeps(time.Second, 60*time.Millisecond, 5*time.Second))
	c, read := dial(t, url)
	send(t, c, ask("t2", "", "q"))
	m := lastFrame(read)
	if m["code"] != "provider_timeout" || !strings.Contains(m["message"].(string), "stopped working") {
		t.Fatalf("got %v", m)
	}
}

func TestATotalLimitStopsAnAIThatKeepsTrickling(t *testing.T) {
	url := startTestWith(t, func(ctx context.Context, _, _ string, onText func(string)) (string, error) {
		for {
			select {
			case <-ctx.Done():
				return "", ctx.Err()
			case <-time.After(10 * time.Millisecond):
				onText("あ") // never silent for long, never finishes
			}
		}
	}, timeoutDeps(time.Second, time.Second, 150*time.Millisecond))
	c, read := dial(t, url)
	send(t, c, ask("t3", "", "q"))
	m := lastFrame(read)
	if m["code"] != "provider_timeout" || !strings.Contains(m["message"].(string), "finish") {
		t.Fatalf("got %v", m)
	}
}

func TestASteadyAnswerWithinTheLimitsIsNotCut(t *testing.T) {
	url := startTestWith(t, func(_ context.Context, _, _ string, onText func(string)) (string, error) {
		for i := 0; i < 5; i++ {
			time.Sleep(30 * time.Millisecond)
			onText("あ")
		}
		return "あああああ", nil
	}, timeoutDeps(500*time.Millisecond, 200*time.Millisecond, 2*time.Second))
	c, read := dial(t, url)
	send(t, c, ask("t4", "", "q"))
	if m := lastFrame(read); m["type"] != "done" {
		t.Fatalf("a normal answer must complete: %v", m)
	}
}

func TestARunningAnswerSaysItIsAlive(t *testing.T) {
	url := startTestWith(t, func(ctx context.Context, _, _ string, _ func(string)) (string, error) {
		select {
		case <-time.After(300 * time.Millisecond):
			return "", nil
		case <-ctx.Done():
			return "", ctx.Err()
		}
	}, func(d *Deps) { d.KeepaliveEvery = 30 * time.Millisecond })
	c, read := dial(t, url)
	send(t, c, ask("t5", "", "q"))
	alive := 0
	for m := read(); m["type"] != "done" && m["type"] != "error"; m = read() {
		if m["type"] == "status" && m["stage"] == "alive" {
			alive++
		}
	}
	if alive < 3 {
		t.Fatalf("expected several keep-alives while the AI is silent, got %d", alive)
	}
}

// withActivity makes the test AI one that reports signs of life.
func withActivity(f AskActiveFunc, tweak func(*Deps)) func(*Deps) {
	return func(d *Deps) {
		d.AskActive = f
		if tweak != nil {
			tweak(d)
		}
	}
}

func TestThinkingWithoutTextIsNotSilence(t *testing.T) {
	url := startTestWith(t, nil, withActivity(func(ctx context.Context, _, _ string, onText func(string), onActivity func(bool)) (string, error) {
		onActivity(false) // the process started
		for i := 0; i < 10; i++ {
			time.Sleep(25 * time.Millisecond)
			onActivity(true) // the model is working, but there is no text yet (250 ms > the 80 ms limit)
		}
		onText("あ")
		return "あ", nil
	}, timeoutDeps(200*time.Millisecond, 80*time.Millisecond, 5*time.Second)))
	c, read := dial(t, url)
	send(t, c, ask("th1", "", "q"))
	if m := lastFrame(read); m["type"] != "done" {
		t.Fatalf("an AI that is thinking must not be cut: %v", m)
	}
}

func TestAProcessWhoseModelNeverBeginsIsStopped(t *testing.T) {
	url := startTestWith(t, nil, withActivity(func(ctx context.Context, _, _ string, _ func(string), onActivity func(bool)) (string, error) {
		for {
			select {
			case <-ctx.Done():
				return "", ctx.Err()
			case <-time.After(20 * time.Millisecond):
				onActivity(false) // alive, but the model never answers (network down, service busy)
			}
		}
	}, timeoutDeps(100*time.Millisecond, time.Second, 5*time.Second)))
	c, read := dial(t, url)
	send(t, c, ask("th2", "", "q"))
	m := lastFrame(read)
	if m["code"] != "provider_timeout" || !strings.Contains(m["message"].(string), "no sign") {
		t.Fatalf("got %v", m)
	}
}

func TestAModelThatWentQuietAfterWorkingIsStopped(t *testing.T) {
	url := startTestWith(t, nil, withActivity(func(ctx context.Context, _, _ string, _ func(string), onActivity func(bool)) (string, error) {
		onActivity(true)
		<-ctx.Done()
		return "", ctx.Err()
	}, timeoutDeps(time.Second, 70*time.Millisecond, 5*time.Second)))
	c, read := dial(t, url)
	send(t, c, ask("th3", "", "q"))
	m := lastFrame(read)
	if m["code"] != "provider_timeout" || !strings.Contains(m["message"].(string), "stopped working") {
		t.Fatalf("got %v", m)
	}
}

func TestTheKeepAliveSaysWhereTheAIIs(t *testing.T) {
	url := startTestWith(t, nil, withActivity(func(ctx context.Context, _, _ string, onText func(string), onActivity func(bool)) (string, error) {
		time.Sleep(150 * time.Millisecond) // starting: nothing yet
		onActivity(true)
		time.Sleep(150 * time.Millisecond) // working: no text
		onText("あ")
		return "あ", nil
	}, func(d *Deps) { d.KeepaliveEvery = 30 * time.Millisecond }))
	c, read := dial(t, url)
	send(t, c, ask("th4", "", "q"))
	phases := map[string]bool{}
	for m := read(); m["type"] != "done" && m["type"] != "error"; m = read() {
		if m["type"] == "status" && m["stage"] == "alive" {
			phases[m["phase"].(string)] = true
			if _, ok := m["idleMs"]; !ok && m["phase"] == "starting" && len(phases) > 1 {
				t.Fatalf("idleMs should accompany the keep-alive: %v", m)
			}
		}
	}
	if !phases["starting"] || !phases["working"] {
		t.Fatalf("keep-alives should report starting and then working, got %v", phases)
	}
}

// terminals counts the frames that end an answer, and says where each one came after the other.
func terminals(frames []msg) (n int, last string) {
	for _, m := range frames {
		if m["type"] == "done" || (m["type"] == "error" && m["seq"] != nil) {
			n++
			last = m["type"].(string) + ":" + fmt.Sprint(m["code"])
		}
	}
	return
}

func TestALateResultAfterACancelIsIgnored(t *testing.T) {
	release := make(chan struct{})
	url := startTest(t, func(ctx context.Context, _, _ string, onText func(string)) (string, error) {
		onText("あ\n")
		<-release
		onText("い")      // after the cancel: must never reach the plugin
		return "あい", nil // …and a successful finish must not add a second ending
	})
	c, read := dial(t, url)
	send(t, c, ask("lc1", "", "q"))
	var got []msg
	for m := read(); m["type"] != "line"; m = read() {
		got = append(got, m)
	}
	send(t, c, map[string]any{"type": "cancel", "id": "lc1"})
	for m := read(); ; m = read() {
		got = append(got, m)
		if m["type"] == "error" && m["code"] == "cancelled" {
			break
		}
	}
	close(release) // the AI now "finishes" anyway
	time.Sleep(150 * time.Millisecond)
	// Everything the run kept, from the first frame: it ends once, with the cancel, and nothing
	// follows that ending.
	c2, _ := dial(t, url)
	send(t, c2, map[string]any{"type": "resume", "id": "lc1", "next": 0})
	var all []msg
	ctx, cancel := context.WithTimeout(context.Background(), 400*time.Millisecond)
	defer cancel()
	for {
		_, b, err := c2.Read(ctx)
		if err != nil {
			break
		}
		var m msg
		_ = json.Unmarshal(b, &m)
		all = append(all, m)
	}
	n, last := terminals(all)
	if n != 1 || last != "error:cancelled" {
		t.Fatalf("an answer ends exactly once, with the cancel: %d, %s\n%v", n, last, all)
	}
	if end := all[len(all)-1]; end["seq"] == nil || end["type"] != "error" {
		t.Fatalf("no frame may follow the ending: %v", all)
	}
}

func TestTheSameIdTwiceAndASecondQuestionAreAnsweredNotStarted(t *testing.T) {
	release := make(chan struct{})
	started := 0
	url := startTest(t, func(ctx context.Context, _, _ string, onText func(string)) (string, error) {
		started++
		onText("あ\n")
		<-release
		return "あ", nil
	})
	defer close(release)
	c, read := dial(t, url)
	send(t, c, ask("d1", "", "q"))
	for m := read(); m["type"] != "line"; m = read() {
	}
	send(t, c, ask("d2", "", "another while d1 runs"))
	var busy msg
	for m := read(); ; m = read() {
		if m["type"] == "error" {
			busy = m
			break
		}
	}
	if busy["code"] != "busy" || busy["id"] != "d2" {
		t.Fatalf("a second question on a busy connection gets busy: %v", busy)
	}
	c2, read2 := dial(t, url)
	send(t, c2, ask("d1", "", "same id again"))
	var dup msg
	for m := read2(); ; m = read2() {
		if m["type"] == "error" {
			dup = m
			break
		}
	}
	if dup["code"] != "bad_request" || dup["message"] != "duplicate id" {
		t.Fatalf("a repeated id is refused: %v", dup)
	}
	if started != 1 {
		t.Fatalf("neither refused question may reach the AI, started %d times", started)
	}
}

func TestTheNewestConnectionReceivesTheAnswer(t *testing.T) {
	release := make(chan struct{})
	url := startTest(t, func(ctx context.Context, _, _ string, onText func(string)) (string, error) {
		onText("あ\n")
		<-release
		onText("い\n")
		return "あい", nil
	})
	c1, read1 := dial(t, url)
	send(t, c1, ask("n1", "", "q"))
	for m := read1(); m["type"] != "line"; m = read1() {
	}
	c2, read2 := dial(t, url) // a second connection takes the answer over while the first is still open
	send(t, c2, map[string]any{"type": "resume", "id": "n1", "next": 1})
	time.Sleep(50 * time.Millisecond)
	close(release)
	var seqs []float64
	for m := read2(); ; m = read2() {
		if m["type"] == "line" || m["type"] == "done" {
			seqs = append(seqs, m["seq"].(float64))
		}
		if m["type"] == "done" {
			break
		}
	}
	if len(seqs) < 2 || seqs[0] != 1 {
		t.Fatalf("the new connection gets the rest from frame 1: %v", seqs)
	}
	// the first connection is no longer the receiver: nothing more arrives for it
	ctx, cancel := context.WithTimeout(context.Background(), 200*time.Millisecond)
	defer cancel()
	for {
		_, b, err := c1.Read(ctx)
		if err != nil {
			break
		}
		var m msg
		_ = json.Unmarshal(b, &m)
		if m["type"] == "line" || m["type"] == "done" {
			t.Fatalf("the replaced connection still receives frames: %v", m)
		}
	}
}

func TestFinishedAnswersAreForgottenAfterTheirRetention(t *testing.T) {
	rs := NewRuns()
	rs.Retention = 30 * time.Millisecond
	quiet := log.New(io.Discard, "", 0)
	first, _ := rs.start("old", func() {}, quiet, 0)
	first.fire(reProviderFail, failInfo{"claude_failed", "x"}) // ends it: FA or FD
	if !first.isFinished() {
		t.Fatal("a failure ends the run")
	}
	time.Sleep(60 * time.Millisecond)
	if _, _ = rs.start("new", func() {}, quiet, 0); rs.get("old") != nil {
		t.Fatal("a finished run past its retention is forgotten by the next start")
	}
	running, _ := rs.start("running", func() {}, quiet, 0)
	time.Sleep(60 * time.Millisecond)
	rs.start("another", func() {}, quiet, 0)
	if rs.get("running") == nil || running.isFinished() {
		t.Fatal("a run that is still working is never forgotten")
	}
}

func TestAKeepAliveIsNotSentToANobody(t *testing.T) {
	// RD × KEEPALIVE is an ignore cell: a run with nobody attached sends nothing and stays RD.
	quiet := log.New(io.Discard, "", 0)
	r := newRun("k", func() {}, rsRa, quiet, time.Hour)
	r.disconnect(&session{}) // a connection that is not the one receiving: not news
	if r.state() != rsRa {
		t.Fatalf("a disconnect from nobody changes nothing, state %s", r.state())
	}
	r.mu.Lock()
	r.m.state = rsRd
	r.mu.Unlock()
	r.fire(reKeepalive, nil)
	if r.state() != rsRd {
		t.Fatal("a keep-alive does not change the state")
	}
}

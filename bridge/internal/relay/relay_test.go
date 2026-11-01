package relay

import (
	"context"
	"encoding/json"
	"io"
	"log"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"

	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/ink"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/securechan"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/server"
)

func TestRoomIDHidesToken(t *testing.T) {
	id := RoomID("secret")
	if len(id) != 32 || strings.Contains(id, "secret") || id == RoomID("other") {
		t.Fatalf("bad room id %q", id)
	}
}

// A plugin session through the relay behaves exactly like a direct LAN session.
func TestSessionThroughRelay(t *testing.T) {
	quiet := log.New(io.Discard, "", 0)
	relaySrv := httptest.NewServer(NewServer(quiet).Handler())
	defer relaySrv.Close()
	base := "ws" + strings.TrimPrefix(relaySrv.URL, "http")

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	// Plugin before the bridge is up: told the bridge is offline.
	c, _, err := websocket.Dial(ctx, ClientURL(base, "secret"), nil)
	if err != nil {
		t.Fatal(err)
	}
	_, _, err = c.Read(ctx)
	if websocket.CloseStatus(err) != StatusBridgeOffline {
		t.Fatalf("want bridge-offline close, got %v", err)
	}

	font := &ink.Font{Em: 100, Glyphs: map[rune]ink.Glyph{'あ': {Advance: 100, Strokes: [][]int16{{50, 0, 50, 100}}}}}
	deps := server.Deps{
		Name: "relayed", Version: "0", Token: "secret", Font: font, Log: quiet,
		Report: func(string, any) {},
		Ask: func(_ context.Context, _, _ string, onText func(string)) (string, error) {
			onText("あ")
			return "あ", nil
		},
	}
	go Connect(ctx, base, "secret", func(ctx context.Context, c *websocket.Conn, peer string) {
		server.ServeConn(ctx, deps, c, peer, true)
	}, quiet)

	// Wait for the bridge to register, then talk to it through the relay (encrypted).
	var got []string
	for attempt := 0; attempt < 50 && len(got) == 0; attempt++ {
		time.Sleep(50 * time.Millisecond)
		got = converse(ctx, t, ClientURL(base, "secret"), true)
	}
	if strings.Join(got, ",") != "welcome,status,status,line,done" {
		t.Fatalf("got %v", got)
	}
	// The relay path refuses the legacy plaintext hello.
	if got := converse(ctx, t, ClientURL(base, "secret"), false); strings.Join(got, ",") != "error" {
		t.Fatalf("plaintext through the relay: got %v", got)
	}
}

// converse asks one question, either over Noise or with the legacy plaintext hello.
func converse(ctx context.Context, t *testing.T, url string, encrypted bool) []string {
	c, _, err := websocket.Dial(ctx, url, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer c.CloseNow()
	c.SetReadLimit(1 << 20)
	send := func(b []byte) error { return c.Write(ctx, websocket.MessageText, b) }
	recv := func() ([]byte, error) { _, b, err := c.Read(ctx); return b, err }
	frames := []any{map[string]any{"type": "hello", "token": "secret", "client": "t"}}
	if encrypted {
		ch, err := securechan.Dial(ctx, c, securechan.PSK("secret"))
		if err != nil {
			return nil // bridge not registered yet: closed as offline
		}
		send = func(b []byte) error { return ch.Write(ctx, b) }
		recv = func() ([]byte, error) { return ch.Read(ctx) }
		frames = nil
	}
	layout := ink.Layout{X: 10, Y: 10, Width: 400, PageHeight: 1000, BottomMargin: 20, CharSize: 40}
	frames = append(frames, map[string]any{"type": "ask", "id": "q", "question": "x", "lang": "ja", "layout": layout})
	for _, f := range frames {
		b, _ := json.Marshal(f)
		if err := send(b); err != nil {
			return nil
		}
	}
	var types []string
	for {
		b, err := recv()
		if err != nil {
			return types
		}
		var m struct{ Type string }
		_ = json.Unmarshal(b, &m)
		types = append(types, m.Type)
		if m.Type == "done" || m.Type == "error" {
			return types
		}
	}
}

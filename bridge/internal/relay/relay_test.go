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
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/protocol"
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
		Report:   func(string, any) {},
		Settings: func() protocol.Settings { return protocol.Settings{} }, SetSettings: func(protocol.Settings) error { return nil },
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

// Anyone can name a new room, so the relay must stop growing at a limit — while a bridge that is
// already registered can still reconnect.
func TestTheRelayRefusesNewRoomsAtItsLimit(t *testing.T) {
	quiet := log.New(io.Discard, "", 0)
	s := NewServer(quiet)
	s.MaxRooms = 2
	srv := httptest.NewServer(s.Handler())
	defer srv.Close()
	base := "ws" + strings.TrimPrefix(srv.URL, "http")
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	bridgeURL := func(token string) string { return base + BridgePath + "?room=" + RoomID(token) }
	var held []*websocket.Conn
	for _, tok := range []string{"a", "b"} {
		c, _, err := websocket.Dial(ctx, bridgeURL(tok), nil)
		if err != nil {
			t.Fatal(err)
		}
		held = append(held, c)
	}
	defer func() {
		for _, c := range held {
			c.CloseNow()
		}
	}()
	time.Sleep(100 * time.Millisecond) // both registered

	third, _, err := websocket.Dial(ctx, bridgeURL("c"), nil)
	if err != nil {
		t.Fatal(err)
	}
	defer third.CloseNow()
	if _, _, err := third.Read(ctx); websocket.CloseStatus(err) != StatusRelayFull {
		t.Fatalf("a third room must be refused with the relay-full code, got %v", err)
	}

	again, _, err := websocket.Dial(ctx, bridgeURL("a"), nil) // an existing room reconnecting
	if err != nil {
		t.Fatal(err)
	}
	defer again.CloseNow()
	readCtx, stop := context.WithTimeout(ctx, 300*time.Millisecond)
	defer stop()
	if _, _, err := again.Read(readCtx); websocket.CloseStatus(err) == StatusRelayFull {
		t.Fatal("a bridge whose room already exists must not be refused")
	}
}

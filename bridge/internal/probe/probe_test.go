package probe

import (
	"context"
	"io"
	"log"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/ink"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/protocol"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/server"
)

func bridge(t *testing.T) string {
	t.Helper()
	srv := httptest.NewServer(server.Handler(server.Deps{
		Name: "probe-test", Version: "9", Token: "secret", Font: &ink.Font{Em: 100, Glyphs: map[rune]ink.Glyph{}},
		Log: log.New(io.Discard, "", 0), Report: func(string, any) {},
		Settings:    func() protocol.Settings { return protocol.Settings{} },
		SetSettings: func(protocol.Settings) error { return nil },
	}))
	t.Cleanup(srv.Close)
	return "ws" + strings.TrimPrefix(srv.URL, "http") + protocol.WSPath
}

func ctx(t *testing.T) context.Context {
	c, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	t.Cleanup(cancel)
	return c
}

func TestProbeReachesABridgeWithTheRightToken(t *testing.T) {
	r, err := Probe(ctx(t), bridge(t), "secret")
	if err != nil {
		t.Fatal(err)
	}
	if r.Bridge != "probe-test" || r.Version != "9" || r.Protocol != protocol.Version || r.Elapsed <= 0 {
		t.Fatalf("got %+v", r)
	}
}

func TestProbeWithTheWrongTokenFailsAtTheHandshake(t *testing.T) {
	_, err := Probe(ctx(t), bridge(t), "not-the-token")
	if err == nil || !strings.Contains(err.Error(), "handshake") && !strings.Contains(err.Error(), "welcome") && !strings.Contains(err.Error(), "connect") {
		t.Fatalf("a wrong token must be an error at the handshake, got %v", err)
	}
}

func TestProbeSaysWhenNothingIsThere(t *testing.T) {
	c, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	if _, err := Probe(c, "ws://127.0.0.1:1/v1", "secret"); err == nil || !strings.Contains(err.Error(), "connect") {
		t.Fatalf("got %v", err)
	}
}

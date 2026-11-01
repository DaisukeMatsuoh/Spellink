package securechan

import (
	"bytes"
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
)

// echoServer accepts one Noise session with key psk and echoes every message back.
func echoServer(t *testing.T, psk []byte) string {
	t.Helper()
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		c, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer c.CloseNow()
		c.SetReadLimit(1 << 20)
		_, msg1, err := c.Read(r.Context())
		if err != nil {
			return
		}
		ch, err := Accept(r.Context(), c, psk, msg1)
		if err != nil {
			_ = c.Close(websocket.StatusPolicyViolation, "unauthorized")
			return
		}
		for {
			m, err := ch.Read(r.Context())
			if err != nil {
				return
			}
			if err := ch.Write(r.Context(), m); err != nil {
				return
			}
		}
	}))
	t.Cleanup(srv.Close)
	return "ws" + strings.TrimPrefix(srv.URL, "http")
}

func dial(t *testing.T, url string) (*websocket.Conn, context.Context) {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	t.Cleanup(cancel)
	c, _, err := websocket.Dial(ctx, url, nil)
	if err != nil {
		t.Fatal(err)
	}
	c.SetReadLimit(1 << 20)
	t.Cleanup(func() { c.CloseNow() })
	return c, ctx
}

func TestRoundTripIncludingChunkedMessages(t *testing.T) {
	url := echoServer(t, PSK("token"))
	c, ctx := dial(t, url)
	ch, err := Dial(ctx, c, PSK("token"))
	if err != nil {
		t.Fatal(err)
	}
	big := bytes.Repeat([]byte("ストローク"), 30000) // ~450 KB: many Noise chunks
	for _, msg := range [][]byte{[]byte(`{"type":"hello"}`), big, {}} {
		if err := ch.Write(ctx, msg); err != nil {
			t.Fatal(err)
		}
		got, err := ch.Read(ctx)
		if err != nil {
			t.Fatal(err)
		}
		if !bytes.Equal(got, msg) {
			t.Fatalf("echo mismatch: %d bytes vs %d", len(got), len(msg))
		}
	}
}

func TestWrongKeyIsRejected(t *testing.T) {
	url := echoServer(t, PSK("token"))
	c, ctx := dial(t, url)
	// With NNpsk0 the responder detects the wrong key on message 1 and closes.
	if _, err := Dial(ctx, c, PSK("other")); err == nil {
		t.Fatal("handshake with the wrong key succeeded")
	}
}

func TestPSKIsDerived(t *testing.T) {
	if len(PSK("a")) != 32 || bytes.Equal(PSK("a"), PSK("b")) {
		t.Fatal("bad PSK derivation")
	}
}

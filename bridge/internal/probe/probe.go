// Package probe checks that a bridge can be reached the way the plugin reaches it: open the
// WebSocket, complete the end-to-end encrypted handshake with the token, and read the welcome.
// `spellink-bridge check` uses it, so a person can tell "the tunnel is up" from "the plugin will
// get through" without a Supernote in hand.
package probe

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"time"

	"github.com/coder/websocket"

	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/protocol"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/securechan"
)

// Result is what a successful probe learned.
type Result struct {
	Bridge   string
	Version  string
	Protocol int
	Elapsed  time.Duration
}

// Probe dials url with the pairing token. It never asks a question.
func Probe(ctx context.Context, url, token string) (Result, error) {
	started := time.Now()
	c, _, err := websocket.Dial(ctx, url, nil)
	if err != nil {
		return Result{}, fmt.Errorf("connect: %w", err)
	}
	defer c.CloseNow()
	ch, err := securechan.Dial(ctx, c, securechan.PSK(token))
	if err != nil {
		// A wrong token surfaces here: the bridge cannot answer the handshake.
		return Result{}, fmt.Errorf("handshake: %w", err)
	}
	b, err := ch.Read(ctx)
	if err != nil {
		return Result{}, fmt.Errorf("welcome: %w", err)
	}
	var w protocol.Welcome
	if err := json.Unmarshal(b, &w); err != nil {
		return Result{}, fmt.Errorf("welcome: %w", err)
	}
	if w.Type != "welcome" {
		var e protocol.Error
		_ = json.Unmarshal(b, &e)
		return Result{}, fmt.Errorf("the bridge answered %q instead of welcome (%s)", w.Type, e.Message)
	}
	if w.Protocol != protocol.Version {
		return Result{}, fmt.Errorf("the bridge speaks protocol %d, this program %d", w.Protocol, protocol.Version)
	}
	if w.Bridge == "" {
		return Result{}, errors.New("welcome without a bridge name")
	}
	_ = c.Close(websocket.StatusNormalClosure, "check done")
	return Result{Bridge: w.Bridge, Version: w.Version, Protocol: w.Protocol, Elapsed: time.Since(started)}, nil
}

package relay

import (
	"context"
	"encoding/json"
	"log"
	"net/url"
	"strings"
	"time"

	"github.com/coder/websocket"
)

// ServeFunc handles one plugin session arriving through the relay.
type ServeFunc func(ctx context.Context, c *websocket.Conn, peer string)

// ClientURL is the address the plugin dials to reach the bridge through relayBase
// (e.g. "wss://relay.example.com").
func ClientURL(relayBase, token string) string {
	return strings.TrimRight(relayBase, "/") + ClientPath + "?room=" + RoomID(token)
}

// Connect keeps the bridge registered with the relay until ctx ends, reconnecting with backoff,
// and serves every session the relay announces.
func Connect(ctx context.Context, relayBase, token string, serve ServeFunc, logger *log.Logger) {
	base := strings.TrimRight(relayBase, "/")
	room := RoomID(token)
	backoff := time.Second
	for ctx.Err() == nil {
		started := time.Now()
		err := connectOnce(ctx, base, room, serve, logger)
		if ctx.Err() != nil {
			return
		}
		if time.Since(started) > time.Minute {
			backoff = time.Second // it was up for a while; retry quickly
		}
		logger.Printf("relay: %v; retrying in %s", err, backoff)
		select {
		case <-time.After(backoff):
		case <-ctx.Done():
			return
		}
		backoff = min(backoff*2, time.Minute)
	}
}

func connectOnce(ctx context.Context, base, room string, serve ServeFunc, logger *log.Logger) error {
	c, _, err := websocket.Dial(ctx, base+BridgePath+"?room="+room, nil)
	if err != nil {
		return err
	}
	defer c.CloseNow()
	logger.Printf("relay: connected to %s", base)

	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	// Pings keep NAT mappings and proxies (e.g. Cloudflare's idle timeout) from dropping us.
	go func() {
		t := time.NewTicker(pingInterval)
		defer t.Stop()
		for {
			select {
			case <-t.C:
				pctx, pcancel := context.WithTimeout(ctx, 10*time.Second)
				err := c.Ping(pctx)
				pcancel()
				if err != nil {
					cancel()
					return
				}
			case <-ctx.Done():
				return
			}
		}
	}()
	for {
		_, b, err := c.Read(ctx)
		if err != nil {
			return err
		}
		var m struct{ Type, ID string }
		if json.Unmarshal(b, &m) != nil || m.Type != "session" || m.ID == "" {
			continue
		}
		go func(sid string) {
			u := base + AcceptPath + "?room=" + room + "&session=" + url.QueryEscape(sid)
			sc, _, err := websocket.Dial(ctx, u, nil)
			if err != nil {
				logger.Printf("relay: accept %s: %v", sid, err)
				return
			}
			defer sc.CloseNow()
			serve(ctx, sc, "relay")
		}(m.ID)
	}
}

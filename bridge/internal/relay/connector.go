package relay

import (
	"context"
	"encoding/json"
	"log"
	"net/url"
	"strings"
	"sync"
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
	ConnectRoom(ctx, relayBase, RoomID(token), serve, logger)
}

// ConnectRoom is Connect for a room id that is already known (one device's secret gives one room).
func ConnectRoom(ctx context.Context, relayBase, room string, serve ServeFunc, logger *log.Logger) {
	base := strings.TrimRight(relayBase, "/")
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

// RoomSet keeps the bridge registered in several rooms at once and follows a set that changes as
// devices are added, paired and revoked (ADR 0016: a device's B and K each have a room).
type RoomSet struct {
	ctx    context.Context
	base   string
	logger *log.Logger
	// run keeps one room registered until its context ends (ConnectRoom; a test replaces it).
	run func(ctx context.Context, base, room string, serve ServeFunc, logger *log.Logger)

	mu    sync.Mutex
	rooms map[string]context.CancelFunc
}

// NewRoomSet returns a set that registers with relayBase until ctx ends.
func NewRoomSet(ctx context.Context, relayBase string, logger *log.Logger) *RoomSet {
	return &RoomSet{ctx: ctx, base: relayBase, logger: logger, run: ConnectRoom, rooms: map[string]context.CancelFunc{}}
}

// Sync makes the registered rooms exactly the keys of want: new rooms are registered, rooms that
// are no longer wanted are left. The serve function of a room that stays is not replaced.
func (r *RoomSet) Sync(want map[string]ServeFunc) {
	r.mu.Lock()
	defer r.mu.Unlock()
	for room, cancel := range r.rooms {
		if _, ok := want[room]; !ok {
			cancel()
			delete(r.rooms, room)
		}
	}
	for room, serve := range want {
		if _, ok := r.rooms[room]; ok {
			continue
		}
		ctx, cancel := context.WithCancel(r.ctx)
		r.rooms[room] = cancel
		go r.run(ctx, r.base, room, serve, r.logger)
	}
}

// Len is how many rooms are registered.
func (r *RoomSet) Len() int {
	r.mu.Lock()
	defer r.mu.Unlock()
	return len(r.rooms)
}

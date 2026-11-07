// Package relay lets a plugin outside the LAN reach a bridge behind NAT. Both sides dial out
// to the relay, which pairs them by room and copies WebSocket frames between them without
// interpreting them (end-to-end encryption is the bridge's and plugin's job, ADR 0007/0010).
//
//	bridge ──/v1/relay/bridge?room=R──▶ relay   (control: one per bridge, long-lived)
//	plugin ──/v1/relay/client?room=R──▶ relay ──{"type":"session","id":S}──▶ bridge
//	bridge ──/v1/relay/accept?room=R&session=S──▶ relay  ⇄ frames ⇄ plugin
package relay

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"io"
	"log"
	"net/http"
	"regexp"
	"sync"
	"time"

	"github.com/coder/websocket"
)

const (
	BridgePath = "/v1/relay/bridge"
	ClientPath = "/v1/relay/client"
	AcceptPath = "/v1/relay/accept"

	// Close codes sent to the plugin.
	StatusBridgeOffline websocket.StatusCode = 4404
	StatusRoomBusy      websocket.StatusCode = 4429
	// StatusRelayFull is sent to a bridge when the relay has no room left for a new one.
	StatusRelayFull websocket.StatusCode = 4503

	maxFrameBytes      = 1 << 20
	maxSessionsPerRoom = 4
	// maxRooms bounds the memory anyone can make the relay use: a room is created by any
	// connection naming a new id, and ids are free to invent.
	maxRooms      = 10000
	acceptTimeout = 10 * time.Second
	pingInterval  = 30 * time.Second
)

var roomRe = regexp.MustCompile(`^[0-9a-f]{32}$`)

// RoomID derives the relay room from the bridge token. The relay only ever sees this hash,
// never the token itself.
func RoomID(token string) string {
	h := sha256.Sum256([]byte("spellink-room-v1:" + token))
	return hex.EncodeToString(h[:16])
}

type pending struct {
	client *websocket.Conn
	done   chan struct{}
}

type room struct {
	ctrl     *websocket.Conn
	ctrlMu   sync.Mutex // serialises control writes
	pending  map[string]*pending
	sessions int
}

// Server is the relay. It keeps everything in memory; restarting it just makes bridges reconnect.
type Server struct {
	Log *log.Logger
	// MaxRooms bounds the number of rooms (default maxRooms).
	MaxRooms int
	mu       sync.Mutex
	rooms    map[string]*room
}

func NewServer(logger *log.Logger) *Server {
	return &Server{Log: logger, MaxRooms: maxRooms, rooms: map[string]*room{}}
}

func (s *Server) Handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /health", func(w http.ResponseWriter, _ *http.Request) {
		s.mu.Lock()
		n := len(s.rooms)
		s.mu.Unlock()
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(map[string]any{"ok": true, "bridges": n})
	})
	mux.HandleFunc(BridgePath, s.handleBridge)
	mux.HandleFunc(ClientPath, s.handleClient)
	mux.HandleFunc(AcceptPath, s.handleAccept)
	return mux
}

func accept(w http.ResponseWriter, r *http.Request) (*websocket.Conn, string, bool) {
	id := r.URL.Query().Get("room")
	if !roomRe.MatchString(id) {
		http.Error(w, "bad room", http.StatusBadRequest)
		return nil, "", false
	}
	c, err := websocket.Accept(w, r, &websocket.AcceptOptions{InsecureSkipVerify: true})
	if err != nil {
		return nil, "", false
	}
	c.SetReadLimit(maxFrameBytes)
	return c, id, true
}

// dropIfIdle forgets a room with no bridge and no sessions. Callers hold s.mu.
func (s *Server) dropIfIdle(id string, rm *room) {
	if rm.ctrl == nil && rm.sessions == 0 && s.rooms[id] == rm {
		delete(s.rooms, id)
	}
}

// handleBridge holds a bridge's control connection open; a newer one for the same room wins.
func (s *Server) handleBridge(w http.ResponseWriter, r *http.Request) {
	c, id, ok := accept(w, r)
	if !ok {
		return
	}
	defer c.CloseNow()
	s.mu.Lock()
	rm := s.rooms[id]
	if rm == nil && len(s.rooms) >= s.MaxRooms {
		s.mu.Unlock()
		s.Log.Printf("relay full: refused a new room")
		_ = c.Close(StatusRelayFull, "relay is full")
		return
	}
	if rm == nil {
		rm = &room{pending: map[string]*pending{}}
		s.rooms[id] = rm
	} else if rm.ctrl != nil {
		_ = rm.ctrl.Close(websocket.StatusGoingAway, "replaced")
	}
	rm.ctrl = c
	s.mu.Unlock()
	s.Log.Printf("bridge up: room %s…", id[:8])

	// Reading keeps pings/pongs flowing and tells us when the bridge goes away.
	for {
		if _, _, err := c.Read(r.Context()); err != nil {
			break
		}
	}
	s.mu.Lock()
	if rm.ctrl == c {
		rm.ctrl = nil
		s.dropIfIdle(id, rm)
	}
	s.mu.Unlock()
	s.Log.Printf("bridge down: room %s…", id[:8])
}

// handleClient asks the room's bridge to open a session and waits until it is piped through.
func (s *Server) handleClient(w http.ResponseWriter, r *http.Request) {
	c, id, ok := accept(w, r)
	if !ok {
		return
	}
	defer c.CloseNow()
	sid := rand.Text()
	p := &pending{client: c, done: make(chan struct{})}

	s.mu.Lock()
	rm := s.rooms[id]
	switch {
	case rm == nil || rm.ctrl == nil:
		s.mu.Unlock()
		_ = c.Close(StatusBridgeOffline, "bridge offline")
		return
	case rm.sessions >= maxSessionsPerRoom:
		s.mu.Unlock()
		_ = c.Close(StatusRoomBusy, "too many sessions")
		return
	}
	rm.sessions++
	rm.pending[sid] = p
	ctrl := rm.ctrl
	s.mu.Unlock()
	defer func() {
		s.mu.Lock()
		delete(rm.pending, sid)
		rm.sessions--
		s.dropIfIdle(id, rm)
		s.mu.Unlock()
	}()

	msg, _ := json.Marshal(map[string]string{"type": "session", "id": sid})
	rm.ctrlMu.Lock()
	err := ctrl.Write(r.Context(), websocket.MessageText, msg)
	rm.ctrlMu.Unlock()
	if err != nil {
		_ = c.Close(StatusBridgeOffline, "bridge offline")
		return
	}
	select {
	case <-p.done:
	case <-time.After(acceptTimeout):
		s.mu.Lock()
		_, stillPending := rm.pending[sid]
		delete(rm.pending, sid)
		s.mu.Unlock()
		if stillPending {
			_ = c.Close(StatusBridgeOffline, "bridge did not answer")
			return
		}
		<-p.done // accepted at the last moment; wait for the pipe to finish
	case <-r.Context().Done():
	}
}

// handleAccept is the bridge's per-session connection; it pipes frames to the waiting plugin.
func (s *Server) handleAccept(w http.ResponseWriter, r *http.Request) {
	sid := r.URL.Query().Get("session")
	c, id, ok := accept(w, r)
	if !ok {
		return
	}
	defer c.CloseNow()
	s.mu.Lock()
	var p *pending
	if rm := s.rooms[id]; rm != nil {
		p = rm.pending[sid]
		delete(rm.pending, sid)
	}
	s.mu.Unlock()
	if p == nil {
		_ = c.Close(websocket.StatusPolicyViolation, "unknown session")
		return
	}
	defer close(p.done)
	pipe(r.Context(), p.client, c)
}

// pipe copies frames both ways until either side closes.
func pipe(ctx context.Context, a, b *websocket.Conn) {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	var wg sync.WaitGroup
	copyOne := func(dst, src *websocket.Conn) {
		defer wg.Done()
		defer cancel()
		for {
			typ, rd, err := src.Reader(ctx)
			if err != nil {
				return
			}
			wr, err := dst.Writer(ctx, typ)
			if err != nil {
				return
			}
			if _, err := io.Copy(wr, rd); err != nil {
				return
			}
			if err := wr.Close(); err != nil {
				return
			}
		}
	}
	wg.Add(2)
	go copyOne(a, b)
	go copyOne(b, a)
	wg.Wait()
	_ = a.Close(websocket.StatusNormalClosure, "")
	_ = b.Close(websocket.StatusNormalClosure, "")
}

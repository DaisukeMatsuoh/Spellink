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
	bucket   *shareBucket // the room's share of bandwidth
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
	// TrustProxy: take the client's address from Cf-Connecting-Ip / X-Forwarded-For. Set it only
	// when the relay is reachable solely through such a proxy or tunnel.
	TrustProxy bool
	// IdleTimeout ends a session in which no frame has passed for this long (default 15 minutes; an
	// answer being written sends a frame every few seconds, so a live session is never idle).
	IdleTimeout time.Duration
	// RoomBytesPerSec and RoomBurst are one room's share of bandwidth, counting both directions
	// (defaults 4 MiB/s and 16 MiB: an answer is hundreds of KB). Beyond it the room is slowed, not cut.
	RoomBytesPerSec, RoomBurst int64
	// Attempts per address per minute, per kind of connection.
	clients, bridges, accepts *limiter
	mu                        sync.Mutex
	rooms                     map[string]*room
}

func NewServer(logger *log.Logger) *Server {
	return &Server{
		Log: logger, MaxRooms: maxRooms, rooms: map[string]*room{},
		IdleTimeout: defaultIdleTimeout, RoomBytesPerSec: defaultRoomBytesPerSec, RoomBurst: defaultRoomBurst,
		// a plugin connects once per question (and when reconnecting); a bridge keeps one control
		// connection and opens one more per plugin session
		clients: newLimiter(60, time.Minute), bridges: newLimiter(12, time.Minute), accepts: newLimiter(120, time.Minute),
	}
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
	if !s.admit(w, r, s.bridges) {
		return
	}
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
		rm = &room{pending: map[string]*pending{}, bucket: newShareBucket(s.RoomBytesPerSec, s.RoomBurst, time.Now)}
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
	if !s.admit(w, r, s.clients) {
		return
	}
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
	if !s.admit(w, r, s.accepts) {
		return
	}
	sid := r.URL.Query().Get("session")
	c, id, ok := accept(w, r)
	if !ok {
		return
	}
	defer c.CloseNow()
	s.mu.Lock()
	var p *pending
	var bk *shareBucket
	if rm := s.rooms[id]; rm != nil {
		p = rm.pending[sid]
		delete(rm.pending, sid)
		bk = rm.bucket
	}
	s.mu.Unlock()
	if p == nil {
		_ = c.Close(websocket.StatusPolicyViolation, "unknown session")
		return
	}
	defer close(p.done)
	pipe(r.Context(), p.client, c, s.IdleTimeout, bk)
}

// pipe copies frames both ways until either side closes, the session has been silent for `idle`, or
// the context ends. The room's bucket slows a room that moves more than its share.
func pipe(ctx context.Context, a, b *websocket.Conn, idle time.Duration, bk *shareBucket) {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	activity := time.AfterFunc(idle, cancel)
	defer activity.Stop()
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
			n, err := io.Copy(wr, rd)
			if err != nil {
				return
			}
			if err := wr.Close(); err != nil {
				return
			}
			activity.Reset(idle)
			if wait := bk.take(n); wait > 0 {
				select {
				case <-time.After(wait):
				case <-ctx.Done():
					return
				}
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

const (
	defaultIdleTimeout     = 15 * time.Minute
	defaultRoomBytesPerSec = 4 << 20
	defaultRoomBurst       = 16 << 20
)

// shareBucket is a token bucket: `rate` bytes per second refill it up to `burst`.
type shareBucket struct {
	mu     sync.Mutex
	rate   float64
	burst  float64
	tokens float64
	last   time.Time
	now    func() time.Time
}

func newShareBucket(rate, burst int64, now func() time.Time) *shareBucket {
	if rate <= 0 {
		rate = defaultRoomBytesPerSec
	}
	if burst <= 0 {
		burst = defaultRoomBurst
	}
	return &shareBucket{rate: float64(rate), burst: float64(burst), tokens: float64(burst), last: now(), now: now}
}

// take spends n bytes and says how long to wait before sending more (0: no need). A nil bucket never makes anyone wait.
func (b *shareBucket) take(n int64) time.Duration {
	if b == nil {
		return 0
	}
	b.mu.Lock()
	defer b.mu.Unlock()
	now := b.now()
	b.tokens = min(b.burst, b.tokens+now.Sub(b.last).Seconds()*b.rate)
	b.last = now
	b.tokens -= float64(n)
	if b.tokens >= 0 {
		return 0
	}
	return time.Duration(-b.tokens / b.rate * float64(time.Second))
}

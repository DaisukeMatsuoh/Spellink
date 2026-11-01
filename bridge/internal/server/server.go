// Package server is the bridge's HTTP/WebSocket front: one WebSocket per plugin session,
// authenticated by a shared token, answering one question at a time.
package server

import (
	"context"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/binary"
	"encoding/json"
	"errors"
	"log"
	"math"
	"net/http"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/coder/websocket"

	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/claude"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/ink"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/protocol"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/securechan"
)

// AskFunc asks the model and streams text deltas through onText.
type AskFunc func(ctx context.Context, question, systemPrompt string, onText func(string)) (string, error)

type Deps struct {
	Name    string
	Version string
	Token   string
	Font    *ink.Font
	Ask     AskFunc
	Report  func(peer string, data any)
	Log     *log.Logger
	// Settings returns the stored user settings; SetSettings changes and persists them.
	Settings    func() protocol.Settings
	SetSettings func(protocol.Settings) error
}

const (
	helloTimeout     = 5 * time.Second
	maxFrameBytes    = 256 << 10
	maxQuestionChars = 4000
	maxPages         = 10
)

// Handler serves GET /health (unauthenticated liveness probe) and the WebSocket at /v1.
func Handler(d Deps) http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /health", func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(map[string]any{"ok": true, "protocol": protocol.Version})
	})
	mux.HandleFunc(protocol.WSPath, func(w http.ResponseWriter, r *http.Request) {
		// The plugin is not a browser, so there is no Origin to check; the token authenticates.
		c, err := websocket.Accept(w, r, &websocket.AcceptOptions{InsecureSkipVerify: true})
		if err != nil {
			return
		}
		// Anything arriving through a proxy or tunnel (e.g. Cloudflare) has crossed the internet:
		// only the end-to-end encrypted handshake is accepted there.
		proxied := r.Header.Get("Cf-Connecting-Ip") != "" || r.Header.Get("X-Forwarded-For") != ""
		ServeConn(r.Context(), d, c, r.RemoteAddr, proxied)
	})
	return mux
}

// ServeConn runs one plugin session on an accepted WebSocket, whether it came in directly on
// the LAN or through the relay. With requireEncryption (relay sessions) only the Noise
// handshake is accepted; on the LAN the legacy plaintext hello still works for now.
func ServeConn(ctx context.Context, d Deps, c *websocket.Conn, peer string, requireEncryption bool) {
	c.SetReadLimit(maxFrameBytes)
	s := &session{
		deps: d, conn: c, peer: peer, expected: sha256.Sum256([]byte(d.Token)),
		requireEncryption: requireEncryption,
	}
	s.run(ctx)
}

type session struct {
	deps     Deps
	conn     *websocket.Conn
	peer     string
	expected [32]byte

	requireEncryption bool
	secure            *securechan.Channel // set once the Noise handshake succeeds

	writeMu sync.Mutex
	askMu   sync.Mutex
	askID   string
	cancel  context.CancelFunc
}

func (s *session) send(ctx context.Context, v any) {
	b, err := json.Marshal(v)
	if err != nil {
		return
	}
	s.writeMu.Lock()
	defer s.writeMu.Unlock()
	if s.secure != nil {
		_ = s.secure.Write(ctx, b)
		return
	}
	_ = s.conn.Write(ctx, websocket.MessageText, b)
}

func (s *session) fail(ctx context.Context, code, msg string) {
	s.send(ctx, protocol.Error{Type: "error", Code: code, Message: msg})
	_ = s.conn.Close(websocket.StatusPolicyViolation, code)
}

func (s *session) read(ctx context.Context) (*protocol.ClientMessage, error) {
	var b []byte
	var err error
	if s.secure != nil {
		b, err = s.secure.Read(ctx)
	} else {
		var typ websocket.MessageType
		typ, b, err = s.conn.Read(ctx)
		if err == nil && typ != websocket.MessageText {
			err = errors.New("frames must be JSON text")
		}
	}
	if err != nil {
		return nil, err
	}
	var m protocol.ClientMessage
	if json.Unmarshal(b, &m) != nil {
		return nil, errors.New("frames must be JSON")
	}
	return &m, nil
}

// authenticate handles the first frame: a binary Noise handshake (authenticated by the
// token-derived key), or — on the LAN only — a plaintext hello carrying the token.
func (s *session) authenticate(ctx context.Context) (client string, ok bool) {
	typ, first, err := s.conn.Read(ctx)
	if err != nil {
		return "", false
	}
	if typ == websocket.MessageBinary {
		ch, err := securechan.Accept(ctx, s.conn, securechan.PSK(s.deps.Token), first)
		if err != nil {
			s.deps.Log.Printf("rejected connection from %s: %v", s.peer, err)
			_ = s.conn.Close(websocket.StatusPolicyViolation, "unauthorized")
			return "", false
		}
		s.secure = ch
		return "noise", true
	}
	var hello protocol.ClientMessage
	if s.requireEncryption || json.Unmarshal(first, &hello) != nil || hello.Type != "hello" {
		s.fail(ctx, "unauthorized", "encrypted handshake expected")
		return "", false
	}
	got := sha256.Sum256([]byte(hello.Token))
	if subtle.ConstantTimeCompare(got[:], s.expected[:]) != 1 {
		s.deps.Log.Printf("rejected connection from %s", s.peer)
		s.fail(ctx, "unauthorized", "bad token")
		return "", false
	}
	return hello.Client + " (plaintext)", true
}

func (s *session) run(parent context.Context) {
	ctx, cancel := context.WithCancel(parent)
	defer cancel()
	defer s.conn.CloseNow()

	helloCtx, helloCancel := context.WithTimeout(ctx, helloTimeout)
	client, ok := s.authenticate(helloCtx)
	helloCancel()
	if !ok {
		return
	}
	s.deps.Log.Printf("connected: %s (%.80s)", s.peer, client)
	s.send(ctx, protocol.Welcome{
		Type: "welcome", Bridge: s.deps.Name, Version: s.deps.Version, Protocol: protocol.Version,
		Settings: s.deps.Settings(),
	})

	for {
		m, err := s.read(ctx)
		if err != nil {
			if websocket.CloseStatus(err) == -1 && ctx.Err() == nil && !errors.Is(err, context.Canceled) {
				s.fail(ctx, "bad_request", err.Error())
			}
			return // closing ctx also cancels a running question
		}
		switch m.Type {
		case "ask":
			s.startAsk(ctx, m)
		case "cancel":
			s.askMu.Lock()
			if s.askID == m.ID && s.cancel != nil {
				s.cancel()
			}
			s.askMu.Unlock()
		case "report":
			s.deps.Report(s.peer, m.Data)
		case "setSettings":
			if m.Settings == nil {
				s.fail(ctx, "bad_request", "settings missing")
				return
			}
			if err := s.deps.SetSettings(*m.Settings); err != nil {
				s.deps.Log.Printf("settings: %v", err)
			}
			s.send(ctx, protocol.SettingsReply{Type: "settings", Settings: s.deps.Settings()})
		default:
			s.fail(ctx, "bad_request", "unknown message type")
			return
		}
	}
}

// ValidLayout checks an untrusted layout from the plugin.
func ValidLayout(l *ink.Layout) bool {
	in := func(v, lo, hi float64) bool { return !math.IsNaN(v) && !math.IsInf(v, 0) && v >= lo && v <= hi }
	const page = 20000
	return l != nil &&
		in(l.PageHeight, 100, page) &&
		in(l.CharSize, 12, 300) &&
		in(l.X, 0, page) &&
		in(l.Y, 0, l.PageHeight) &&
		in(l.Width, l.CharSize, page) &&
		in(l.BottomMargin, 0, l.PageHeight) &&
		l.MaxPages >= 0 && l.MaxPages <= maxPages &&
		in(l.ContinueTop, 0, l.PageHeight)
}

func seedFrom(id string) uint32 {
	h := sha256.Sum256([]byte(id))
	return binary.LittleEndian.Uint32(h[:4])
}

// budget is the answer length (in full-width characters) to ask the model for.
func budget(l ink.Layout) int {
	b := ink.Capacity(l)
	if l.MaxPages > 1 {
		b = max(b, 200)
	}
	return min(max(b, 60), 600)
}

func (s *session) startAsk(ctx context.Context, m *protocol.ClientMessage) {
	id := m.ID
	if len(id) > 64 {
		id = id[:64]
	}
	q := strings.TrimSpace(m.Question)
	if id == "" || q == "" || len([]rune(q)) > maxQuestionChars || !ValidLayout(m.Layout) {
		s.send(ctx, protocol.Error{Type: "error", ID: id, Code: "bad_request", Message: "invalid ask"})
		return
	}
	lang := "ja"
	if m.Lang == "en" {
		lang = "en"
	}

	s.askMu.Lock()
	if s.cancel != nil {
		s.askMu.Unlock()
		s.send(ctx, protocol.Error{Type: "error", ID: id, Code: "busy", Message: "another question is in progress"})
		return
	}
	askCtx, cancel := context.WithCancel(ctx)
	s.askID, s.cancel = id, cancel
	s.askMu.Unlock()

	go func() {
		defer func() {
			cancel()
			s.askMu.Lock()
			s.askID, s.cancel = "", nil
			s.askMu.Unlock()
		}()
		s.answer(ctx, askCtx, cancel, id, q, lang, *m.Layout)
	}()
}

func (s *session) answer(ctx, askCtx context.Context, stop context.CancelFunc, id, q, lang string, layout ink.Layout) {
	started := time.Now()
	if st := s.deps.Settings(); st.LineTilt {
		// Positive degrees rise to the right, i.e. y decreases as x grows.
		layout.Slope = -math.Tan(st.TiltDegrees * math.Pi / 180)
	}
	composer := ink.NewComposer(s.deps.Font, layout, seedFrom(id))
	emit := func(lines []ink.Line) {
		for _, l := range lines {
			s.send(ctx, protocol.Line{Type: "line", ID: id, Line: l})
		}
	}
	writing := false
	s.deps.Log.Printf("ask %s: %.80s", id, strings.Join(strings.Fields(q), " "))
	s.send(ctx, protocol.Status{Type: "status", ID: id, Stage: "thinking"})

	text, err := s.deps.Ask(askCtx, q, claude.SystemPrompt(lang, budget(layout)), func(delta string) {
		if !writing {
			writing = true
			s.send(ctx, protocol.Status{Type: "status", ID: id, Stage: "writing"})
		}
		emit(composer.Push(delta))
		if composer.Truncated {
			stop() // stop paying for tokens that can no longer fit
		}
	})
	if err != nil && !composer.Truncated {
		code, msg := "claude_failed", err.Error()
		if askCtx.Err() != nil {
			code, msg = "cancelled", "cancelled"
		}
		if len(msg) > 500 {
			msg = msg[:500]
		}
		s.deps.Log.Printf("ask %s %s: %s", id, code, msg)
		s.send(ctx, protocol.Error{Type: "error", ID: id, Code: code, Message: msg})
		return
	}
	emit(composer.Finish())
	missing := make([]string, 0, len(composer.Missing))
	for r := range composer.Missing {
		missing = append(missing, string(r))
	}
	sort.Strings(missing)
	elapsed := time.Since(started).Milliseconds()
	s.deps.Log.Printf("ask %s done in %d ms, %d page(s)%s", id, elapsed, composer.Pages,
		map[bool]string{true: " (truncated)"}[composer.Truncated])
	s.send(ctx, protocol.Done{
		Type: "done", ID: id, Text: text, Missing: missing, Truncated: composer.Truncated,
		Pages: composer.Pages, ElapsedMs: elapsed,
	})
}

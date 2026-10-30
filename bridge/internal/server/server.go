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

	"github.com/DaisukeMatsuoh/Cloute/bridge/internal/claude"
	"github.com/DaisukeMatsuoh/Cloute/bridge/internal/ink"
	"github.com/DaisukeMatsuoh/Cloute/bridge/internal/protocol"
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
	expected := sha256.Sum256([]byte(d.Token))
	mux.HandleFunc(protocol.WSPath, func(w http.ResponseWriter, r *http.Request) {
		// The plugin is not a browser, so there is no Origin to check; the token authenticates.
		c, err := websocket.Accept(w, r, &websocket.AcceptOptions{InsecureSkipVerify: true})
		if err != nil {
			return
		}
		c.SetReadLimit(maxFrameBytes)
		s := &session{deps: d, conn: c, peer: r.RemoteAddr, expected: expected}
		s.run(r.Context())
	})
	return mux
}

type session struct {
	deps     Deps
	conn     *websocket.Conn
	peer     string
	expected [32]byte

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
	_ = s.conn.Write(ctx, websocket.MessageText, b)
}

func (s *session) fail(ctx context.Context, code, msg string) {
	s.send(ctx, protocol.Error{Type: "error", Code: code, Message: msg})
	_ = s.conn.Close(websocket.StatusPolicyViolation, code)
}

func (s *session) read(ctx context.Context) (*protocol.ClientMessage, error) {
	typ, b, err := s.conn.Read(ctx)
	if err != nil {
		return nil, err
	}
	var m protocol.ClientMessage
	if typ != websocket.MessageText || json.Unmarshal(b, &m) != nil {
		return nil, errors.New("frames must be JSON text")
	}
	return &m, nil
}

func (s *session) run(parent context.Context) {
	ctx, cancel := context.WithCancel(parent)
	defer cancel()
	defer s.conn.CloseNow()

	helloCtx, helloCancel := context.WithTimeout(ctx, helloTimeout)
	hello, err := s.read(helloCtx)
	helloCancel()
	if err != nil || hello.Type != "hello" {
		s.fail(ctx, "unauthorized", "hello expected")
		return
	}
	got := sha256.Sum256([]byte(hello.Token))
	if subtle.ConstantTimeCompare(got[:], s.expected[:]) != 1 {
		s.deps.Log.Printf("rejected connection from %s", s.peer)
		s.fail(ctx, "unauthorized", "bad token")
		return
	}
	s.deps.Log.Printf("connected: %s (%.80s)", s.peer, hello.Client)
	s.send(ctx, protocol.Welcome{Type: "welcome", Bridge: s.deps.Name, Version: s.deps.Version, Protocol: protocol.Version})

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

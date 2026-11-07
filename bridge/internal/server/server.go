// Package server is the bridge's HTTP/WebSocket front: one WebSocket per plugin session,
// authenticated by a shared token, answering one question at a time.
package server

import (
	"cmp"
	"context"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/binary"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"math"
	"net/http"
	"sort"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/coder/websocket"

	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/claude"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/history"
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
	// History remembers each note's conversation; nil (or HistoryChars 0) means every question
	// stands alone. HistoryChars bounds how much earlier conversation is sent with a question.
	History      *history.Store
	HistoryChars int
	// Runs holds the answers in flight; share one between the direct and the relay entrance so
	// a plugin can resume on either. Nil gets a private table per Handler/ServeConn call.
	Runs *Runs
	// FirstOutput, Gap and Total bound the AI's silence (0: defaults 30 s / 20 s / 120 s).
	FirstOutput, Gap, Total time.Duration
	// KeepaliveEvery is how often a running answer tells the plugin it is alive (0: 5 s).
	KeepaliveEvery time.Duration
	// OrphanGrace is how long an answer is still produced with no plugin connected (0: default).
	OrphanGrace time.Duration
}

const (
	helloTimeout     = 5 * time.Second
	maxFrameBytes    = 256 << 10
	maxQuestionChars = 4000
	maxNoteKeyChars  = 1024
	maxPages         = 10
)

// Handler serves GET /health (unauthenticated liveness probe) and the WebSocket at /v1.
func Handler(d Deps) http.Handler {
	if d.Runs == nil {
		d.Runs = NewRuns()
	}
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
	if d.Runs == nil {
		d.Runs = NewRuns()
	}
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

	writeMu  sync.Mutex
	writeErr atomic.Bool // the first failed write has been logged
	askMu    sync.Mutex
	current  *run   // the answer this connection is waiting for, if any
	attached []*run // every run this connection received frames of, to detach on close
}

func (s *session) send(ctx context.Context, v any) {
	b, err := json.Marshal(v)
	if err != nil {
		return
	}
	s.sendRaw(ctx, b)
}

func (s *session) sendRaw(ctx context.Context, b []byte) {
	s.writeMu.Lock()
	defer s.writeMu.Unlock()
	var werr error
	if s.secure != nil {
		werr = s.secure.Write(ctx, b)
	} else {
		werr = s.conn.Write(ctx, websocket.MessageText, b)
	}
	// A failed write means the plugin did not get this frame; "done" in the log must not be
	// mistaken for "delivered" when the connection died underneath.
	if werr != nil && s.writeErr.CompareAndSwap(false, true) {
		s.deps.Log.Printf("write to %s failed: %v", s.peer, werr)
	}
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
	defer s.detachAll()

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
			s.deps.Log.Printf("disconnected: %s: %v", s.peer, err)
			if websocket.CloseStatus(err) == -1 && ctx.Err() == nil && !errors.Is(err, context.Canceled) {
				s.fail(ctx, "bad_request", err.Error())
			}
			return // closing ctx also cancels a running question
		}
		switch m.Type {
		case "ask":
			s.startAsk(ctx, m)
		case "cancel":
			if r := s.deps.Runs.get(m.ID); r != nil {
				r.cancel()
			}
		case "resume":
			s.resume(ctx, m.ID, m.Next)
		case "clearHistory":
			n, err := s.deps.History.Clear()
			if err != nil {
				s.deps.Log.Printf("history: clear: %v", err)
			}
			s.send(ctx, protocol.HistoryCleared{Type: "historyCleared", Notes: n})
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

// detachAll is called when the connection ends: its answers keep being produced for a while
// so the plugin can reconnect and fetch them (ADR 0013).
func (s *session) detachAll() {
	s.askMu.Lock()
	runs := s.attached
	s.attached = nil
	s.askMu.Unlock()
	grace := cmp.Or(s.deps.OrphanGrace, defaultOrphanGrace)
	for _, r := range runs {
		r.detach(s, grace)
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
		in(l.ContinueTop, 0, l.PageHeight) &&
		in(l.StartLimit, 0, l.PageHeight)
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
	if s.current != nil {
		s.askMu.Unlock()
		s.send(ctx, protocol.Error{Type: "error", ID: id, Code: "busy", Message: "another question is in progress"})
		return
	}
	// The answer must not die with this connection: a dropped tunnel would otherwise lose it.
	// It stops on an explicit cancel, or when nobody comes back for it (orphanGrace).
	askCtx, cancel := context.WithCancel(context.WithoutCancel(ctx))
	r, ok := s.deps.Runs.start(id, cancel)
	if !ok {
		s.askMu.Unlock()
		cancel()
		s.send(ctx, protocol.Error{Type: "error", ID: id, Code: "bad_request", Message: "duplicate id"})
		return
	}
	s.current = r
	s.attached = append(s.attached, r)
	s.askMu.Unlock()
	r.attach(ctx, s, 0)

	// While the answer is being produced, say so: the plugin takes "nothing at all for a while"
	// to mean the connection is dead.
	go func() {
		t := time.NewTicker(cmp.Or(s.deps.KeepaliveEvery, 5*time.Second))
		defer t.Stop()
		for {
			select {
			case <-askCtx.Done():
				return
			case <-t.C:
				r.notify(protocol.Status{Type: "status", ID: id, Stage: "alive"})
			}
		}
	}()

	go func() {
		defer func() {
			r.finish()
			cancel()
			s.askMu.Lock()
			if s.current == r {
				s.current = nil
			}
			s.askMu.Unlock()
		}()
		s.answer(askCtx, r, id, q, lang, m.Note, *m.Layout)
	}()
}

// resume hands a reconnected plugin the frames of answer `id` from number `next` on, and
// keeps sending the rest live.
func (s *session) resume(ctx context.Context, id string, next int) {
	r := s.deps.Runs.get(id)
	if r == nil {
		s.send(ctx, protocol.Error{Type: "error", ID: id, Code: "unknown_ask", Message: "this answer is no longer kept"})
		return
	}
	s.askMu.Lock()
	s.attached = append(s.attached, r)
	if !r.isFinished() {
		s.current = r
	}
	s.askMu.Unlock()
	s.deps.Log.Printf("resume %s from frame %d for %s", id, next, s.peer)
	r.attach(ctx, s, next)
}

func (s *session) answer(askCtx context.Context, r *run, id, q, lang, note string, layout ink.Layout) {
	started := time.Now()
	st := s.deps.Settings()
	if st.LineTilt {
		// Positive degrees rise to the right, i.e. y decreases as x grows.
		layout.Slope = -math.Tan(st.TiltDegrees * math.Pi / 180)
	}
	layout.SpacingFull = float64(st.FullWidthSpacing) / 100
	layout.SpacingHalf = float64(st.HalfWidthSpacing) / 100
	composer := ink.NewComposer(s.deps.Font, layout, seedFrom(id))
	emit := func(lines []ink.Line) {
		for _, l := range lines {
			r.publish(func(seq int) any { return protocol.Line{Type: "line", ID: id, Seq: seq, Line: l} })
		}
	}
	writing := false
	s.deps.Log.Printf("ask %s: %.80s", id, strings.Join(strings.Fields(q), " "))
	r.notify(protocol.Status{Type: "status", ID: id, Stage: "thinking"})

	if len([]rune(note)) > maxNoteKeyChars {
		note = "" // not a real note key: ask without memory rather than guess
	}
	earlier, err := s.deps.History.Recent(note, s.deps.HistoryChars)
	if err != nil {
		s.deps.Log.Printf("history: read: %v", err) // the question is still worth answering
	}

	// The AI must not stay silent: a first output, then pieces without long pauses, all within a
	// total (bridge-run spec, R13). Whichever limit hits first stops it.
	first, gap, total := cmp.Or(s.deps.FirstOutput, 30*time.Second), cmp.Or(s.deps.Gap, 20*time.Second), cmp.Or(s.deps.Total, 120*time.Second)
	var timedOut atomic.Pointer[string]
	expire := func(why string) func() {
		return func() {
			if timedOut.CompareAndSwap(nil, &why) {
				r.cancel()
			}
		}
	}
	totalTimer := time.AfterFunc(total, expire(fmt.Sprintf("the AI did not finish within %d s", int(total.Seconds()))))
	firstTimer := time.AfterFunc(first, expire(fmt.Sprintf("the AI gave no output within %d s", int(first.Seconds()))))
	var gapTimer *time.Timer // from the first output on, replaces firstTimer
	gapWhy := fmt.Sprintf("the AI paused for more than %d s", int(gap.Seconds()))
	defer func() {
		totalTimer.Stop()
		firstTimer.Stop()
		if gapTimer != nil {
			gapTimer.Stop()
		}
	}()

	text, err := s.deps.Ask(askCtx, history.Transcript(earlier, q, lang), claude.SystemPrompt(lang, budget(layout)), func(delta string) {
		if !writing {
			writing = true
			firstTimer.Stop()
			gapTimer = time.AfterFunc(gap, expire(gapWhy))
			s.deps.Log.Printf("ask %s first output after %d ms", id, time.Since(started).Milliseconds())
			r.notify(protocol.Status{Type: "status", ID: id, Stage: "writing"})
		} else if timedOut.Load() == nil {
			gapTimer.Reset(gap) // a late piece must not revive an answer that was already stopped
		}
		emit(composer.Push(delta))
		if composer.Truncated {
			r.cancel() // stop paying for tokens that can no longer fit
		}
	})
	if err != nil && !composer.Truncated {
		code, msg := "claude_failed", err.Error()
		if why := timedOut.Load(); why != nil {
			code, msg = "provider_timeout", *why
		} else if askCtx.Err() != nil {
			code, msg = "cancelled", "cancelled"
		}
		if len(msg) > 500 {
			msg = msg[:500]
		}
		s.deps.Log.Printf("ask %s %s: %s", id, code, msg)
		r.publish(func(seq int) any {
			return protocol.Error{Type: "error", ID: id, Seq: &seq, Code: code, Message: msg}
		})
		return
	}
	emit(composer.Finish())
	if s.deps.HistoryChars > 0 && strings.TrimSpace(text) != "" {
		if err := s.deps.History.Append(note, history.Turn{At: time.Now().UTC(), Q: q, A: text}); err != nil {
			s.deps.Log.Printf("history: write: %v", err)
		}
	}
	missing := make([]string, 0, len(composer.Missing))
	for m := range composer.Missing {
		missing = append(missing, string(m))
	}
	sort.Strings(missing)
	elapsed := time.Since(started).Milliseconds()
	s.deps.Log.Printf("ask %s done in %d ms, %d page(s)%s", id, elapsed, composer.Pages,
		map[bool]string{true: " (truncated)"}[composer.Truncated])
	r.publish(func(seq int) any {
		return protocol.Done{
			Type: "done", ID: id, Seq: seq, Text: text, Missing: missing, Truncated: composer.Truncated,
			Pages: composer.Pages, ElapsedMs: elapsed,
		}
	})
}

package server

import (
	"cmp"
	"context"
	"encoding/json"
	"strings"
	"sync"
	"time"

	"github.com/coder/websocket"

	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/ink"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/protocol"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/securechan"
)

// A device session opened with B is the pairing channel (ADR 0016): it may send one pairRequest
// and, after it was granted, one pairAck. Anything else — an ask above all — ends it.
const (
	// defaultPairRequestWait is how long a pairing channel may stay silent before its pairRequest.
	defaultPairRequestWait = 10 * time.Second
	// defaultPairingMax bounds a pairing channel's whole life (the window and the grant's wait are shorter).
	defaultPairingMax = 5 * time.Minute
)

// Sessions tracks the live sessions of each device, so that revoking a device cuts them off.
type Sessions struct {
	mu   sync.Mutex
	byID map[string]map[*session]struct{}
}

// NewSessions returns an empty table.
func NewSessions() *Sessions { return &Sessions{byID: map[string]map[*session]struct{}{}} }

func (t *Sessions) add(id string, s *session) {
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.byID[id] == nil {
		t.byID[id] = map[*session]struct{}{}
	}
	t.byID[id][s] = struct{}{}
}

func (t *Sessions) remove(id string, s *session) {
	t.mu.Lock()
	defer t.mu.Unlock()
	delete(t.byID[id], s)
	if len(t.byID[id]) == 0 {
		delete(t.byID, id)
	}
}

// CloseDevice ends every session of the device (it was revoked or reissued).
func (t *Sessions) CloseDevice(id string) {
	t.mu.Lock()
	live := make([]*session, 0, len(t.byID[id]))
	for s := range t.byID[id] {
		live = append(live, s)
	}
	t.mu.Unlock()
	for _, s := range live {
		go s.closeWith("device_removed") // the close handshake can take seconds; do not make the caller wait
	}
}

// authenticateDevice accepts only the Noise handshake, with the device's K (a normal session) or B
// (the pairing channel). K is tried first: a device that has it never needs B again.
func (s *session) authenticateDevice(ctx context.Context, typ websocket.MessageType, first []byte) (string, bool) {
	reject := func(why string) (string, bool) {
		s.deps.Log.Printf("rejected device connection from %s: %s", s.peer, why)
		_ = s.conn.Close(websocket.StatusPolicyViolation, "unauthorized")
		return "", false
	}
	if s.deps.Devices == nil || typ != websocket.MessageBinary {
		return reject("a device must open with the encrypted handshake")
	}
	b, k, ok := s.deps.Devices.Secrets(s.deviceID)
	if !ok {
		return reject("unknown or finished device")
	}
	if k != "" {
		if ch, err := securechan.Accept(ctx, s.conn, securechan.PSK(k), first); err == nil {
			if err := s.deps.Devices.ConnectK(s.deviceID); err != nil {
				return reject(err.Error())
			}
			s.secure = ch
			s.deps.Sessions.add(s.deviceID, s)
			return "device " + s.deviceID, true
		}
	}
	if b != "" {
		if ch, err := securechan.Accept(ctx, s.conn, securechan.PSK(b), first); err == nil {
			s.secure, s.pairing = ch, true
			s.deps.Sessions.add(s.deviceID, s)
			return "pairing " + s.deviceID, true
		}
	}
	return reject("handshake failed")
}

// runPairing serves the pairing channel until it is granted, refused or the device leaves.
func (s *session) runPairing(parent context.Context) {
	ctx, cancel := context.WithTimeout(parent, cmp.Or(s.deps.PairingMax, defaultPairingMax))
	defer cancel()
	requested := false
	for {
		readCtx, stop := ctx, func() {}
		if !requested {
			readCtx, stop = context.WithTimeout(ctx, cmp.Or(s.deps.PairRequestWait, defaultPairRequestWait))
		}
		m, err := s.read(readCtx)
		stop()
		if err != nil {
			s.deps.Log.Printf("pairing channel %s closed: %v", s.peer, err)
			return
		}
		switch m.Type {
		case "pairRequest":
			if requested {
				s.fail(ctx, "bad_request", "one pairRequest per channel")
				return
			}
			if m.ID != s.deviceID { // a channel opened for one device must not pair another
				s.fail(ctx, "bad_request", "the request is for another device")
				return
			}
			requested = true
			if err := s.deps.Devices.Request(m.ID, m.Gen, m.Code, s); err != nil {
				// the ledger has told the device (Deny) and closed the channel
				s.deps.Log.Printf("pairRequest from %s refused: %v", s.peer, err)
				return
			}
			s.sendBanner(ctx, m)
		case "pairAck":
			if m.ID != s.deviceID {
				s.fail(ctx, "bad_request", "the ack is for another device")
				return
			}
			if err := s.deps.Devices.Ack(m.ID, m.Gen); err != nil {
				s.deps.Log.Printf("pairAck from %s refused: %v", s.peer, err)
			}
			return // the device reconnects with K
		default:
			s.fail(ctx, "bad_request", "only pairing is possible on this channel")
			return
		}
	}
}

// Grant implements devices.Conn: it sends K inside the encrypted channel.
func (s *session) Grant(key string, endpoints []string) error {
	if endpoints == nil {
		endpoints = []string{}
	}
	b, err := json.Marshal(protocol.PairGrant{Type: "pairGrant", Key: key, Endpoints: endpoints})
	if err != nil {
		return err
	}
	return s.sendRaw(context.Background(), b)
}

// Deny implements devices.Conn: it says why and closes the channel.
func (s *session) Deny(reason string) {
	s.send(context.Background(), protocol.PairDenied{Type: "pairDenied", Reason: reason})
	go s.closeWith("pair_denied") // called from another session's goroutine: must not wait for this peer
}

func (s *session) closeWith(reason string) {
	_ = s.conn.Close(websocket.StatusPolicyViolation, reason)
}

const (
	maxBannerRunes   = 120
	maxBannerStrokes = 3000
)

// sendBanner writes the phrase the device asked for in the bridge's handwriting and sends the strokes.
// It is decoration, so anything wrong with the request (no phrase, a layout that is not sane, no
// font, a phrase that draws nothing) means no banner, never a failed pairing.
func (s *session) sendBanner(ctx context.Context, m *protocol.ClientMessage) {
	text := strings.TrimSpace(m.Banner)
	if text == "" || len([]rune(text)) > maxBannerRunes || s.deps.Font == nil || !ValidLayout(m.Layout) {
		return
	}
	layout := *m.Layout
	layout.MaxPages = 1
	composer := ink.NewComposer(s.deps.Font, layout, seedFrom(m.ID))
	lines := append(composer.Push(text), composer.Finish()...)
	var strokes []ink.Stroke
	for _, l := range lines {
		if l.Page == 0 {
			strokes = append(strokes, l.Strokes...)
		}
	}
	if len(strokes) == 0 || len(strokes) > maxBannerStrokes {
		return
	}
	s.send(ctx, protocol.PairBanner{Type: "pairBanner", Strokes: strokes})
}

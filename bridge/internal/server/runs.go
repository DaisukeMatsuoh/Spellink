package server

import (
	"cmp"
	"context"
	"encoding/json"
	"sync"
	"time"
)

// A run is one answer, from the question until it has been delivered. It outlives the WebSocket
// that asked (ADR 0013): the frames it produces are numbered and kept, so a plugin whose
// connection dropped can reconnect, say "I have everything before frame N" and get the rest.
// Nothing is guessed or retried: the plugin names the id and the number it is missing.
type run struct {
	id     string
	cancel context.CancelFunc

	mu       sync.Mutex
	frames   [][]byte // line / done / error frames, in order; frame i carries "seq": i
	finished bool
	finishAt time.Time
	// Signs of life from the AI (see sign / vitals): the keep-alive reports them.
	vmu   sync.Mutex
	phase string
	last  time.Time

	to     *session // where new frames go now; nil while the plugin is disconnected
	toCtx  context.Context
	orphan *time.Timer
}

// phaseRank orders the phases an answer goes through; it never goes back.
var phaseRank = map[string]int{"starting": 0, "working": 1, "writing": 2}

// sign records a sign of life; a non-empty phase moves the answer forward to it.
func (r *run) sign(phase string) {
	r.vmu.Lock()
	defer r.vmu.Unlock()
	r.last = time.Now()
	if phase != "" && phaseRank[phase] > phaseRank[r.phase] {
		r.phase = phase
	}
}

// vitals is where the AI is and how long since it last showed a sign of life.
func (r *run) vitals() (phase string, idle time.Duration) {
	r.vmu.Lock()
	defer r.vmu.Unlock()
	return cmp.Or(r.phase, "starting"), time.Since(r.last)
}

// publish numbers a frame, keeps it and sends it to the plugin if one is attached.
func (r *run) publish(frame func(seq int) any) {
	r.mu.Lock()
	defer r.mu.Unlock()
	b, err := json.Marshal(frame(len(r.frames)))
	if err != nil {
		return
	}
	r.frames = append(r.frames, b)
	if r.to != nil {
		r.to.sendRaw(r.toCtx, b)
	}
}

// notify sends a transient frame (progress). It is not kept: a plugin that reconnects only
// needs the answer.
func (r *run) notify(v any) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.to != nil {
		r.to.send(r.toCtx, v)
	}
}

func (r *run) finish() {
	r.mu.Lock()
	r.finished, r.finishAt = true, time.Now()
	r.mu.Unlock()
}

// attach makes s the receiver, after replaying the frames from number `next` on. Replay and
// live frames cannot interleave or repeat: both happen under the lock.
func (r *run) attach(ctx context.Context, s *session, next int) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.orphan != nil {
		r.orphan.Stop()
		r.orphan = nil
	}
	for i := max(next, 0); i < len(r.frames); i++ {
		s.sendRaw(ctx, r.frames[i])
	}
	r.to, r.toCtx = s, ctx
}

// detach is called when s's connection ended. A run nobody comes back for within grace is
// cancelled so it stops spending the user's Claude usage.
func (r *run) detach(s *session, grace time.Duration) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.to != s {
		return
	}
	r.to, r.toCtx = nil, nil
	if !r.finished && r.orphan == nil {
		r.orphan = time.AfterFunc(grace, r.cancel)
	}
}

// Runs is the bridge's table of answers in flight and recently finished.
type Runs struct {
	mu sync.Mutex
	m  map[string]*run
}

func NewRuns() *Runs { return &Runs{m: map[string]*run{}} }

const (
	runRetention = 10 * time.Minute // how long a finished answer can still be fetched
	maxRuns      = 16
)

// defaultOrphanGrace is how long an answer keeps being produced with no plugin connected.
const defaultOrphanGrace = 30 * time.Second

func (rs *Runs) get(id string) *run {
	rs.mu.Lock()
	defer rs.mu.Unlock()
	return rs.m[id]
}

// start registers a new run, forgetting old finished ones. It reports false for a repeated id.
func (rs *Runs) start(id string, cancel context.CancelFunc) (*run, bool) {
	rs.mu.Lock()
	defer rs.mu.Unlock()
	if _, dup := rs.m[id]; dup {
		return nil, false
	}
	for k, r := range rs.m {
		r.mu.Lock()
		old := r.finished && time.Since(r.finishAt) > runRetention
		r.mu.Unlock()
		if old || len(rs.m) >= maxRuns && r.isFinished() {
			delete(rs.m, k)
		}
	}
	r := &run{id: id, cancel: cancel, phase: "starting", last: time.Now()}
	rs.m[id] = r
	return r, true
}

func (r *run) isFinished() bool {
	r.mu.Lock()
	defer r.mu.Unlock()
	return r.finished
}

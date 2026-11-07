package server

import (
	"cmp"
	"context"
	"encoding/json"
	"log"
	"sync"
	"time"

	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/ink"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/protocol"
)

// A run is one answer, from the question until it has been delivered. It outlives the WebSocket
// that asked (ADR 0013): the frames it produces are numbered and kept, so a plugin whose
// connection dropped can reconnect, say "I have everything before frame N" and get the rest.
// Nothing is guessed or retried: the plugin names the id and the number it is missing.
//
// What happens when is the table in docs/spec/bridge-run.md (runtable_gen.go, run by
// runMachine). An event is fired with run.fire; the cell names the action and the next state; the
// actions below are the handlers. Every handler runs with the run's lock held.

// runMachine looks up the cell for (state, event), moves, and calls the action. Like the
// plugin's machine it is the only place that changes `state`.
type runMachine struct {
	state    runState
	handlers map[runAction]func(arg any)
	log      func(format string, args ...any)
}

func (m *runMachine) dispatch(ev runEvent, arg any, spent bool) {
	cell, ok := runTable[m.state][ev]
	if !ok {
		m.log("run.unknown: state %s event %s", m.state, ev)
		return
	}
	switch cell.kind {
	case cellNever:
		// The table says this cannot happen here. It did: record it loudly, change nothing.
		m.log("run.never: state %s event %s (note %s)", m.state, ev, cell.note)
	case cellIgnore:
		if cell.action != "" {
			m.handlers[cell.action](arg)
		}
	case cellDo:
		m.move(cell.action, cell.next, arg)
	case cellGuard:
		if spent {
			m.move(cell.spent.action, cell.spent.next, arg)
		} else {
			m.move(cell.more.action, cell.more.next, arg)
		}
	}
}

func (m *runMachine) move(action runAction, next runState, arg any) {
	m.state = next // first, so an action that fires further events sees the new state
	m.handlers[action](arg)
}

// ruleN is the table's row for an ID with no run (state N). The registry applies it.
func ruleN(ev runEvent) runCell { return runTable[rsN][ev] }

type run struct {
	id     string
	cancel context.CancelFunc // stops the AI
	log    *log.Logger
	grace  time.Duration

	mu     sync.Mutex
	m      runMachine
	frames [][]byte // line / done / error frames, in order; frame i carries "seq": i
	// finishAt is when the answer was completed (for the retention).
	finishAt time.Time
	to       *session // where new frames go now; nil while the plugin is disconnected
	toCtx    context.Context
	orphan   *time.Timer
	sendErr  bool // the first failed write has been logged

	// Signs of life from the AI (see sign / vitals): the keep-alive reports them.
	vmu   sync.Mutex
	phase string
	last  time.Time
}

// Payloads of events.
type (
	resumeArg struct {
		ctx  context.Context
		s    *session
		next int
	}
	// reply is what the error actions (E1–E3) answer to.
	reply struct {
		ctx context.Context
		s   *session
		id  string
	}
	// doneInfo completes an answer; remember is called once, to keep the exchange in the note's
	// conversation memory.
	doneInfo struct {
		text      string
		missing   []string
		truncated bool
		pages     int
		elapsedMs int64
		remember  func()
	}
	failInfo struct{ code, message string }
)

func newRun(id string, cancel context.CancelFunc, start runState, logger *log.Logger, grace time.Duration) *run {
	r := &run{id: id, cancel: cancel, log: logger, grace: cmp.Or(grace, defaultOrphanGrace), phase: "starting", last: time.Now()}
	r.m = runMachine{state: start, handlers: r.handlers(), log: func(f string, a ...any) { logger.Printf(f, a...) }}
	return r
}

// fire reports an event to the run's table.
func (r *run) fire(ev runEvent, arg any) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.m.dispatch(ev, arg, false)
}

func (r *run) state() runState {
	r.mu.Lock()
	defer r.mu.Unlock()
	return r.m.state
}

func (r *run) isFinished() bool {
	st := r.state()
	return st == rsFa || st == rsFd
}

// disconnect tells the run that session s's connection ended; only the session that is
// receiving it matters (one already replaced by a newer connection is not news).
func (r *run) disconnect(s *session) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.to == s {
		r.m.dispatch(reDisconnect, s, false)
	}
}

func (r *run) handlers() map[runAction]func(any) {
	return map[runAction]func(any){
		raR1: func(any) {}, // the run was created by the registry (Runs.start)
		raR2: func(any) { // CANCEL
			r.cancel()
			r.publishLocked(r.cancelledFrame)
		},
		raR3: func(any) { // DISCONNECT while the AI is working: wait a while for the plugin
			r.to, r.toCtx = nil, nil
			if r.orphan == nil {
				r.orphan = time.AfterFunc(r.grace, func() { r.fire(reGrace, nil) })
			}
		},
		raR4: func(arg any) { // RESUME: replay from `next`, then the newest connection receives
			a := arg.(resumeArg)
			r.stopOrphan()
			for i := max(a.next, 0); i < len(r.frames); i++ {
				r.sendLocked(a.ctx, a.s, r.frames[i])
			}
			r.to, r.toCtx = a.s, a.ctx
		},
		raR5: func(arg any) { // DELTA
			for _, l := range arg.([]ink.Line) {
				l := l
				r.publishLocked(func(seq int) any { return protocol.Line{Type: "line", ID: r.id, Seq: seq, Line: l} })
			}
		},
		raR6: func(any) { // GRACE: nobody came back
			r.cancel()
			r.publishLocked(r.cancelledFrame)
		},
		raR7: func(any) { r.to, r.toCtx = nil, nil }, // DISCONNECT after the answer is complete
		raR8: func(arg any) { r.complete(arg.(doneInfo)) },
		raR9: func(any) {}, // CLEAR: the memory is cleared by the session, whatever runs exist
		raR10: func(arg any) { // PROVIDER_FAIL
			f := arg.(failInfo)
			r.publishLocked(func(seq int) any {
				return protocol.Error{Type: "error", ID: r.id, Seq: &seq, Code: f.code, Message: f.message}
			})
			r.finishAt = time.Now()
		},
		raR11: func(arg any) { // FULL: no more pages; stop paying for text that cannot be written
			r.cancel()
			d := arg.(doneInfo)
			d.truncated = true
			r.complete(d)
		},
		raR12: func(arg any) { // SEND_FAIL: the frames are kept, so a reconnect gets them
			if !r.sendErr {
				r.sendErr = true
				r.log.Printf("write for run %s failed: %v", r.id, arg)
			}
		},
		raR13: func(arg any) { // HANG
			r.cancel()
			why, _ := arg.(string)
			r.publishLocked(func(seq int) any {
				return protocol.Error{Type: "error", ID: r.id, Seq: &seq, Code: "provider_timeout", Message: why}
			})
			r.finishAt = time.Now()
		},
		raR14: func(any) { r.stopOrphan() }, // SWEEP: the registry forgets a run that reached N
		raR15: func(any) { // KEEPALIVE
			phase, idle := r.vitals()
			r.notifyLocked(protocol.Status{Type: "status", ID: r.id, Stage: "alive", Phase: phase, IdleMs: idle.Milliseconds()})
		},
		raE1: func(arg any) {
			a := arg.(reply)
			a.s.send(a.ctx, protocol.Error{Type: "error", ID: a.id, Code: "unknown_ask", Message: "this answer is no longer kept"})
		},
		raE2: func(arg any) {
			a := arg.(reply)
			a.s.send(a.ctx, protocol.Error{Type: "error", ID: a.id, Code: "bad_request", Message: "duplicate id"})
		},
		raE3: func(arg any) {
			a := arg.(reply)
			a.s.send(a.ctx, protocol.Error{Type: "error", ID: a.id, Code: "busy", Message: "another question is in progress"})
		},
	}
}

func (r *run) cancelledFrame(seq int) any {
	return protocol.Error{Type: "error", ID: r.id, Seq: &seq, Code: "cancelled", Message: "cancelled"}
}

func (r *run) complete(d doneInfo) {
	if d.remember != nil {
		d.remember()
	}
	r.publishLocked(func(seq int) any {
		return protocol.Done{
			Type: "done", ID: r.id, Seq: seq, Text: d.text, Missing: d.missing, Truncated: d.truncated,
			Pages: d.pages, ElapsedMs: d.elapsedMs,
		}
	})
	r.finishAt = time.Now()
}

func (r *run) stopOrphan() {
	if r.orphan != nil {
		r.orphan.Stop()
		r.orphan = nil
	}
}

// publishLocked numbers a frame, keeps it and sends it to the plugin if one is attached. The
// caller holds r.mu.
func (r *run) publishLocked(frame func(seq int) any) {
	b, err := json.Marshal(frame(len(r.frames)))
	if err != nil {
		return
	}
	r.frames = append(r.frames, b)
	if r.to != nil {
		r.sendLocked(r.toCtx, r.to, b)
	}
}

// sendLocked writes one frame; a failed write is the SEND_FAIL event (the frame stays kept).
func (r *run) sendLocked(ctx context.Context, s *session, b []byte) {
	if err := s.sendRaw(ctx, b); err != nil {
		r.m.dispatch(reSendFail, err, false)
	}
}

// notifyLocked sends a transient frame (progress). It is not kept: a plugin that reconnects only
// needs the answer.
func (r *run) notifyLocked(v any) {
	if r.to == nil {
		return
	}
	b, err := json.Marshal(v)
	if err != nil {
		return
	}
	r.sendLocked(r.toCtx, r.to, b)
}

// notify sends a progress frame while the answer is running.
func (r *run) notify(v any) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.m.state == rsRa {
		r.notifyLocked(v)
	}
}

// attachFirst makes s the receiver of a new run (R1: the first connection of the answer).
func (r *run) attachFirst(ctx context.Context, s *session) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.to, r.toCtx = s, ctx
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

// Runs is the bridge's table of answers in flight and recently finished.
type Runs struct {
	mu sync.Mutex
	m  map[string]*run
	// Retention is how long a finished answer can still be fetched; Max bounds how many are kept.
	Retention time.Duration
	Max       int
}

func NewRuns() *Runs {
	return &Runs{m: map[string]*run{}, Retention: 10 * time.Minute, Max: 16}
}

// defaultOrphanGrace is how long an answer keeps being produced with no plugin connected.
const defaultOrphanGrace = 30 * time.Second

func (rs *Runs) get(id string) *run {
	rs.mu.Lock()
	defer rs.mu.Unlock()
	return rs.m[id]
}

// start registers a new run (the table's N × ASK cell says where it begins) after sweeping old
// finished ones. For an id that already has a run it returns that run, and nothing is created.
func (rs *Runs) start(id string, cancel context.CancelFunc, logger *log.Logger, grace time.Duration) (created, existing *run) {
	rs.mu.Lock()
	defer rs.mu.Unlock()
	if r, dup := rs.m[id]; dup {
		return nil, r
	}
	rs.sweepLocked()
	cell := ruleN(reAsk)
	r := newRun(id, cancel, cell.next, logger, grace)
	rs.m[id] = r
	return r, nil
}

// sweepLocked fires SWEEP at finished runs that are past their retention (or beyond the cap);
// the ones the table sends to N are forgotten.
func (rs *Runs) sweepLocked() {
	for id, r := range rs.m {
		r.mu.Lock()
		finished := r.m.state == rsFa || r.m.state == rsFd
		old := finished && time.Since(r.finishAt) > rs.Retention
		if old || len(rs.m) >= rs.Max && finished {
			r.m.dispatch(reSweep, nil, false)
		}
		forgotten := r.m.state == rsN
		r.mu.Unlock()
		if forgotten {
			delete(rs.m, id)
		}
	}
}

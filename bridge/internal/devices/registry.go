package devices

import (
	"bytes"
	"crypto/rand"
	"errors"
	"fmt"
	"io"
	"log"
	"sync"
	"time"
)

// Why a request for pairing is refused (the `reason` of a pairDenied frame; pairing.md, C).
const (
	DenyCodeInUse     = "code_in_use"
	DenyPairBusy      = "pair_busy"
	DenyGrantPending  = "grant_pending"
	DenyWindowClosed  = "window_closed"
	DenyExpired       = "expired"
	DenySuperseded    = "superseded"
	DenyUnknownDevice = "unknown_device"
)

// What the owner (the command line, the pairing page) is told when an operation does not apply.
var (
	ErrUnknownDevice  = errors.New("no such device")
	ErrNameTaken      = errors.New("a device with this name already exists")
	ErrBadName        = errors.New("a device name is 1 to 64 characters without control characters")
	ErrFull           = errors.New("the ledger is full")
	ErrNoPending      = errors.New("no device is showing a code")
	ErrBadCode        = errors.New("a pairing code is exactly eight digits")
	ErrCodeRejected   = errors.New("the code was not accepted")
	ErrNotEligible    = errors.New("this device cannot connect with that secret in its current state")
	ErrNotPersisted   = errors.New("the change could not be written to the ledger")
	ErrAlreadyRevoked = errors.New("the device is already revoked")
	ErrRevoked        = errors.New("the device is revoked; add a new one")
	ErrExpired        = errors.New("the first secret has expired; reissue the device")
	ErrStillValid     = errors.New("revoke the device before forgetting it")
	ErrBadRequest     = errors.New("malformed pairing request")
)

// Conn is a device's connection as the ledger sees it: it can be told it is granted K and it can be refused.
type Conn interface {
	// Grant sends K (and where to reach the bridge) to the device. It is called outside the ledger's lock.
	Grant(key string, endpoints []string) error
	// Deny refuses the request with one of the Deny reasons and closes the connection.
	Deny(reason string)
}

// Hooks connect the ledger to the rest of the bridge. Every one may be nil. They are called
// outside the ledger's lock.
type Hooks struct {
	// Endpoints returns the URLs a device should use with secret k (it is sent in pairGrant).
	Endpoints func(id, k string) []string
	// Changed is called after the set of valid secrets changed (rooms on the relay must follow).
	Changed func()
	// CloseAll ends every live session of the device (it was revoked, reissued, or its K replaced).
	CloseAll func(id string)
}

// Config holds the ledger's limits. Zero values take the defaults of ADR 0016.
type Config struct {
	Window      time.Duration // how long requests wait for the owner (default 90 s)
	TTL         time.Duration // how long B stays valid (default 24 h)
	AckTimeout  time.Duration // how long to wait for the device to confirm it has K (default 30 s)
	MaxPending  int           // waiting requests per device (default 4)
	MaxAttempts int           // wrong codes per window (default 5)
	Now         func() time.Time
	Rand        io.Reader
	Log         *log.Logger
}

func (c Config) withDefaults() Config {
	if c.Window <= 0 {
		c.Window = 90 * time.Second
	}
	if c.TTL <= 0 {
		c.TTL = 24 * time.Hour
	}
	if c.AckTimeout <= 0 {
		c.AckTimeout = 30 * time.Second
	}
	if c.MaxPending <= 0 {
		c.MaxPending = 4
	}
	if c.MaxAttempts <= 0 {
		c.MaxAttempts = 5
	}
	if c.Now == nil {
		c.Now = time.Now
	}
	if c.Rand == nil {
		c.Rand = rand.Reader
	}
	if c.Log == nil {
		c.Log = log.New(io.Discard, "", 0)
	}
	return c
}

// lastSeenEvery limits how often a device's last-connection time is written to the ledger.
const lastSeenEvery = time.Minute

type pending struct {
	code string
	conn Conn
}

// rec is a record plus what only lives in memory.
type rec struct {
	Record
	state     devState
	prev      devState // the state before the move that is running (an action sees the new state in `state`)
	pending   []*pending
	windowEnd time.Time
	failures  int
	ackEnd    time.Time
}

// snapshot is what a failed action restores (ADR 0016, P7 in pairing.md).
type snapshot struct {
	rec       rec
	pending   []*pending
	listed    bool
	listIndex int
}

// Registry is the ledger with its state machine. It is safe for concurrent use.
type Registry struct {
	cfg   Config
	store Store
	hooks Hooks

	mu      sync.Mutex
	recs    map[string]*rec
	order   []string
	saved   []byte
	posts   []func()
	changed bool
}

// action is one cell's work. It changes the record in memory and returns what must be done outside
// the lock once the change is saved (sending, closing, telling others).
type action func(r *Registry, rc *rec, arg any) (post []func(), err error)

// New reads the ledger (the RESTART event of the table). A ledger file that cannot be read at all
// is an error; records that cannot be used are left out and listed in `skipped`.
func New(cfg Config, store Store, hooks Hooks) (reg *Registry, skipped []string, err error) {
	r := &Registry{cfg: cfg.withDefaults(), store: store, hooks: hooks, recs: map[string]*rec{}}
	data, err := store.Load()
	if err != nil {
		return nil, nil, fmt.Errorf("reading the device ledger: %w", err)
	}
	if data == nil {
		return r, nil, nil
	}
	recs, skipped, err := decode(data)
	if err != nil {
		return nil, nil, err
	}
	r.saved = data
	for _, rd := range recs {
		rc := &rec{Record: rd, state: devState(rd.State)}
		r.recs[rd.ID] = rc
		r.order = append(r.order, rd.ID)
		// RESTART writes only when a record had to be normalised (a leftover B dropped). If that
		// write fails the bridge still starts: memory is right, the file is fixed by the next write.
		if _, err := r.fire(rc, deRestart, nil, false); err != nil {
			r.cfg.Log.Printf("devices: could not rewrite the ledger at start: %v", err)
		}
	}
	r.cfg.Log.Printf("devices: %d record(s) loaded, %d skipped", len(recs), len(skipped))
	return r, skipped, nil
}

func (r *Registry) rnd(b []byte) error {
	_, err := io.ReadFull(r.cfg.Rand, b)
	return err
}

func (r *Registry) now() time.Time { return r.cfg.Now() }

// deadline is when B stops being valid.
func (r *Registry) deadline(rc *rec) time.Time { return rc.IssuedAt.Add(r.cfg.TTL) }

type outcome int

const (
	moved   outcome = iota // the cell's action ran
	ignored                // the table says: happens, nothing to do
	never                  // the table says: cannot happen here (recorded)
)

func keepsOnError(a devAction) bool { return a == daP13 }

// fire is the one place where a record's state changes. `spent` picks the branch of a guarded
// cell (the caller decides it before the action runs; pairing.md, "条件つきのセル").
func (r *Registry) fire(rc *rec, ev devEvent, arg any, spent bool) (outcome, error) {
	cell, ok := devTable[rc.state][ev]
	if !ok { // the table is complete (the tests check it); this is a bug, not an input
		r.cfg.Log.Printf("devices.unknown: state %s event %s", rc.state, ev)
		return never, nil
	}
	var mv devMove
	switch cell.kind {
	case cellNever:
		r.cfg.Log.Printf("devices.never: state %s event %s (note %s) device %s", rc.state, ev, cell.note, rc.ID)
		return never, nil
	case cellIgnore:
		return ignored, nil
	case cellDo:
		mv = devMove{cell.action, cell.next}
	default:
		mv = cell.more
		if spent {
			mv = cell.spent
		}
	}
	snap := r.snap(rc)
	from := rc.state
	rc.prev, rc.state = from, mv.next
	post, err := actions[mv.action](r, rc, arg)
	if err == nil {
		err = r.save()
	}
	if err != nil {
		if keepsOnError(mv.action) {
			r.posts = append(r.posts, post...)
			r.changed = true
			return moved, fmt.Errorf("%w: %v", ErrNotPersisted, err)
		}
		r.restore(rc, snap)
		return moved, err
	}
	r.posts = append(r.posts, post...)
	if from != mv.next {
		r.changed = true
		r.cfg.Log.Printf("devices.state: %s %s -[%s]-> %s (%s)", rc.ID, from, ev, mv.next, mv.action)
	}
	// An action that returns to I after B's time is up must not leave an expired B valid.
	if (mv.action == daP8 || mv.action == daP11) && rc.state == dsI && !r.now().Before(r.deadline(rc)) {
		return r.fire(rc, deTtl, nil, false)
	}
	return moved, nil
}

func (r *Registry) snap(rc *rec) snapshot {
	s := snapshot{rec: *rc, pending: append([]*pending(nil), rc.pending...)}
	for i, id := range r.order {
		if id == rc.ID {
			s.listed, s.listIndex = true, i
		}
	}
	return s
}

func (r *Registry) restore(rc *rec, s snapshot) {
	cur := rc.ID // who is listed now (a new record has an id the snapshot does not know)
	*rc = s.rec
	rc.pending = s.pending
	at := -1
	for i, id := range r.order {
		if id == cur {
			at = i
		}
	}
	switch {
	case s.listed && at < 0:
		r.order = append(r.order[:s.listIndex], append([]string{rc.ID}, r.order[s.listIndex:]...)...)
		r.recs[rc.ID] = rc
	case !s.listed && at >= 0:
		r.order = append(r.order[:at], r.order[at+1:]...)
		delete(r.recs, cur)
	}
}

// save writes the ledger when it differs from what is stored. Waiting requests are not stored: a
// record in Q is stored as issued.
func (r *Registry) save() error {
	recs := make([]Record, 0, len(r.order))
	for _, id := range r.order {
		rc := r.recs[id]
		rd := rc.Record
		rd.State = string(rc.state)
		if rc.state == dsQ {
			rd.State = string(dsI)
		}
		recs = append(recs, rd)
	}
	data, err := encode(recs)
	if err != nil {
		return err
	}
	if bytes.Equal(data, r.saved) {
		return nil
	}
	if err := r.store.Save(data); err != nil {
		return err
	}
	r.saved = data
	return nil
}

// locked runs f under the lock, then (outside it) the side effects the actions asked for.
func (r *Registry) locked(f func() error) error {
	r.mu.Lock()
	err := f()
	posts, changed := r.posts, r.changed
	r.posts, r.changed = nil, false
	r.mu.Unlock()
	for _, p := range posts {
		p()
	}
	if changed && r.hooks.Changed != nil {
		r.hooks.Changed()
	}
	return err
}

func (r *Registry) get(id string) (*rec, error) {
	rc, ok := r.recs[id]
	if !ok {
		return nil, ErrUnknownDevice
	}
	return rc, nil
}

func (r *Registry) byName(name string) *rec {
	for _, id := range r.order {
		if r.recs[id].Name == name {
			return r.recs[id]
		}
	}
	return nil
}

func (r *Registry) deny(rc *rec, reason string) (post []func()) {
	for _, p := range rc.pending {
		c := p.conn
		post = append(post, func() { c.Deny(reason) })
	}
	rc.pending = nil
	rc.windowEnd = time.Time{}
	return post
}

func (r *Registry) closeAll(id string) []func() {
	if r.hooks.CloseAll == nil {
		return nil
	}
	return []func(){func() { r.hooks.CloseAll(id) }}
}

// actions is the handler for every action of the table (a test checks that none is missing). It is
// filled in init because the handlers call back into the registry.
var actions map[devAction]action

func init() {
	actions = map[devAction]action{
		daP1: func(r *Registry, rc *rec, arg any) ([]func(), error) {
			name := arg.(string)
			if !ValidName(name) {
				return nil, ErrBadName
			}
			if len(r.order) >= MaxDevices {
				return nil, ErrFull
			}
			var id string
			for tries := 0; ; tries++ {
				var err error
				if id, err = newID(r.rnd); err != nil {
					return nil, err
				}
				if _, taken := r.recs[id]; !taken {
					break
				}
				if tries >= 3 {
					return nil, errors.New("could not find a free device id")
				}
			}
			b, err := newSecret(r.rnd)
			if err != nil {
				return nil, err
			}
			rc.Record = Record{ID: id, Name: name, Gen: 1, B: b, IssuedAt: r.now()}
			r.recs[id] = rc
			r.order = append(r.order, id)
			return nil, nil
		},
		daP2: func(r *Registry, rc *rec, arg any) ([]func(), error) {
			req := arg.(*request)
			var post []func()
			if rc.prev == dsI {
				rc.windowEnd = r.now().Add(r.cfg.Window)
				rc.failures = 0
			}
			if i := rc.indexOfCode(req.code); i >= 0 {
				other := rc.pending[i].conn
				rc.pending = append(rc.pending[:i], rc.pending[i+1:]...)
				post = append(post, func() { other.Deny(DenyCodeInUse) }, func() { req.conn.Deny(DenyCodeInUse) })
			} else if len(rc.pending) >= r.cfg.MaxPending {
				post = append(post, func() { req.conn.Deny(DenyPairBusy) })
			} else {
				rc.pending = append(rc.pending, &pending{code: req.code, conn: req.conn})
			}
			if len(rc.pending) == 0 {
				rc.windowEnd = time.Time{}
			}
			return post, nil
		},
		daP3: func(r *Registry, rc *rec, arg any) ([]func(), error) {
			c := arg.(*request).conn
			return []func(){func() { c.Deny(DenyGrantPending) }}, nil
		},
		daP5: func(r *Registry, rc *rec, arg any) ([]func(), error) {
			c := arg.(Conn)
			for i, p := range rc.pending {
				if p.conn == c {
					rc.pending = append(rc.pending[:i], rc.pending[i+1:]...)
					break
				}
			}
			if len(rc.pending) == 0 {
				rc.windowEnd = time.Time{}
			}
			return nil, nil
		},
		daP6: func(r *Registry, rc *rec, arg any) ([]func(), error) {
			chosen := arg.(*pending)
			k, err := newSecret(r.rnd)
			if err != nil {
				return nil, err
			}
			var post []func()
			for _, p := range rc.pending {
				if p != chosen {
					c := p.conn
					post = append(post, func() { c.Deny(DenySuperseded) })
				}
			}
			rc.pending = nil
			rc.windowEnd = time.Time{}
			rc.K = k
			rc.ackEnd = r.now().Add(r.cfg.AckTimeout)
			id, gen := rc.ID, rc.Gen
			var eps []string
			if r.hooks.Endpoints != nil {
				eps = r.hooks.Endpoints(id, k)
			}
			post = append(post, func() {
				if err := chosen.conn.Grant(k, eps); err != nil {
					r.cfg.Log.Printf("devices: sending K to %s failed: %v", id, err)
					_ = r.locked(func() error { return r.timeoutGrant(id, gen) })
				}
			})
			return post, nil
		},
		daP7: func(r *Registry, rc *rec, arg any) ([]func(), error) {
			rc.failures++
			if rc.failures >= r.cfg.MaxAttempts {
				return r.deny(rc, DenyWindowClosed), nil
			}
			return nil, nil
		},
		daP8: func(r *Registry, rc *rec, arg any) ([]func(), error) {
			return r.deny(rc, DenyWindowClosed), nil
		},
		daP9: func(r *Registry, rc *rec, arg any) ([]func(), error) {
			rc.B = ""
			rc.ackEnd = time.Time{}
			rc.LastSeen = r.now()
			return nil, nil
		},
		daP10: func(r *Registry, rc *rec, arg any) ([]func(), error) {
			if now := r.now(); now.Sub(rc.LastSeen) >= lastSeenEvery {
				rc.LastSeen = now
			}
			return nil, nil
		},
		daP11: func(r *Registry, rc *rec, arg any) ([]func(), error) {
			rc.K = ""
			rc.ackEnd = time.Time{}
			return nil, nil
		},
		daP12: func(r *Registry, rc *rec, arg any) ([]func(), error) {
			rc.B = ""
			return r.deny(rc, DenyExpired), nil
		},
		daP13: func(r *Registry, rc *rec, arg any) ([]func(), error) {
			rc.B, rc.K = "", ""
			rc.RevokedAt = r.now()
			rc.ackEnd = time.Time{}
			post := r.deny(rc, DenyUnknownDevice)
			return append(post, r.closeAll(rc.ID)...), nil
		},
		daP14: func(r *Registry, rc *rec, arg any) ([]func(), error) {
			b, err := newSecret(r.rnd)
			if err != nil {
				return nil, err
			}
			post := r.deny(rc, DenySuperseded)
			rc.Gen++
			rc.B, rc.K = b, ""
			rc.IssuedAt = r.now()
			rc.RevokedAt = time.Time{}
			rc.failures = 0
			rc.ackEnd = time.Time{}
			return append(post, r.closeAll(rc.ID)...), nil
		},
		daP15: func(r *Registry, rc *rec, arg any) ([]func(), error) {
			rc.pending, rc.windowEnd, rc.failures = nil, time.Time{}, 0
			rc.ackEnd = time.Time{}
			if rc.state == dsG {
				rc.ackEnd = r.now().Add(r.cfg.AckTimeout)
			}
			return nil, nil
		},
		daP17: func(r *Registry, rc *rec, arg any) ([]func(), error) {
			for i, id := range r.order {
				if id == rc.ID {
					r.order = append(r.order[:i], r.order[i+1:]...)
					break
				}
			}
			delete(r.recs, rc.ID)
			return nil, nil
		},
		daP18: func(r *Registry, rc *rec, arg any) ([]func(), error) {
			out := arg.(*PackageData)
			*out = PackageData{ID: rc.ID, Name: rc.Name, Gen: rc.Gen, B: rc.B, Window: r.cfg.Window}
			return nil, nil
		},
	}
}

func (rc *rec) indexOfCode(code string) int {
	for i, p := range rc.pending {
		if p.code == code {
			return i
		}
	}
	return -1
}

// request is a valid pairRequest on its way into the table.
type request struct {
	code string
	conn Conn
}

// PackageData is what the .snplg for a device carries (REPACK).
type PackageData struct {
	ID     string
	Name   string
	Gen    int
	B      string // empty once the device is paired (it keeps K itself)
	Window time.Duration
}

// timeoutGrant is the ACK_TIMEOUT event for a grant that did not get through (or was not
// confirmed in time). It does nothing if the device has moved on (an old timer).
func (r *Registry) timeoutGrant(id string, gen int) error {
	rc, ok := r.recs[id]
	if !ok || rc.Gen != gen || rc.state != dsG {
		return nil
	}
	_, err := r.fire(rc, deAckTimeout, nil, false)
	return err
}

// ---- the events, as the rest of the bridge calls them ----

// Add is ISSUE: a new device record with its first secret.
func (r *Registry) Add(name string) (id string, err error) {
	err = r.locked(func() error {
		if existing := r.byName(name); existing != nil {
			_, _ = r.fire(existing, deIssue, name, false) // the table's ×d: nothing changes
			return ErrNameTaken
		}
		rc := &rec{state: dsN}
		if _, err := r.fire(rc, deIssue, name, false); err != nil {
			return err
		}
		id = rc.ID
		return nil
	})
	return id, err
}

// Request is CONN_B: a device connected with B and sent a pairRequest.
func (r *Registry) Request(id string, gen int, code string, c Conn) error {
	if !ValidID(id) || !ValidCode(code) || gen < 1 {
		c.Deny(DenyUnknownDevice)
		return ErrBadRequest
	}
	return r.locked(func() error {
		rc, ok := r.recs[id]
		if !ok || rc.Gen != gen {
			r.posts = append(r.posts, func() { c.Deny(DenyUnknownDevice) })
			return ErrUnknownDevice
		}
		req := &request{code: code, conn: c}
		spent := (rc.state == dsI || rc.state == dsQ) && rc.indexOfCode(code) >= 0 && len(rc.pending) == 1
		out, err := r.fire(rc, deConnB, req, spent)
		if out != moved || err != nil {
			r.posts = append(r.posts, func() { c.Deny(DenyUnknownDevice) })
		}
		return err
	})
}

// Disconnect is DISCONNECT: a connection that was waiting for the owner closed.
func (r *Registry) Disconnect(c Conn) {
	_ = r.locked(func() error {
		for _, id := range r.order {
			rc := r.recs[id]
			for _, p := range rc.pending {
				if p.conn == c {
					spent := len(rc.pending) == 1
					_, err := r.fire(rc, deDisconnect, c, spent)
					return err
				}
			}
		}
		return nil
	})
}

// Confirm is CONFIRM_OK or CONFIRM_BAD: the owner typed a code. It returns the name of the
// device that is now granted. A code that matches no waiting request, or more than one, is
// refused without saying which devices are waiting.
func (r *Registry) Confirm(code string) (name string, err error) {
	return r.confirm("", code)
}

// ConfirmFor is Confirm for a code that came with the device's id (the QR code the device shows
// carries both): only that device's requests are looked at, and a wrong code counts against that
// device alone.
func (r *Registry) ConfirmFor(id, code string) (name string, err error) {
	if !ValidID(id) {
		return "", ErrUnknownDevice
	}
	return r.confirm(id, code)
}

func (r *Registry) confirm(onlyID, code string) (name string, err error) {
	if !ValidCode(code) {
		return "", ErrBadCode
	}
	err = r.locked(func() error {
		var waiting []*rec
		var hit *rec
		var hitReq *pending
		hits := 0
		if onlyID != "" {
			if _, err := r.get(onlyID); err != nil {
				return err
			}
		}
		for _, id := range r.order {
			rc := r.recs[id]
			if rc.state != dsQ || (onlyID != "" && id != onlyID) {
				continue
			}
			waiting = append(waiting, rc)
			for _, p := range rc.pending {
				if p.code == code {
					hit, hitReq = rc, p
					hits++
				}
			}
		}
		if len(waiting) == 0 {
			return ErrNoPending
		}
		if hits == 1 {
			if _, err := r.fire(hit, deConfirmOk, hitReq, false); err != nil {
				return err
			}
			name = hit.Name
			return nil
		}
		for _, rc := range waiting {
			if _, err := r.fire(rc, deConfirmBad, nil, rc.failures+1 >= r.cfg.MaxAttempts); err != nil {
				return err
			}
		}
		return ErrCodeRejected
	})
	return name, err
}

// Ack is ACK: the device says it has stored K.
func (r *Registry) Ack(id string, gen int) error {
	return r.locked(func() error {
		rc, err := r.get(id)
		if err != nil || rc.Gen != gen {
			return ErrUnknownDevice
		}
		_, err = r.fire(rc, deAck, nil, false)
		return err
	})
}

// ConnectK is CONN_K: a session was authenticated with K. In G this also counts as the device's
// confirmation that it has K.
func (r *Registry) ConnectK(id string) error {
	return r.locked(func() error {
		rc, err := r.get(id)
		if err != nil {
			return err
		}
		out, err := r.fire(rc, deConnK, nil, false)
		if out != moved && err == nil {
			err = ErrNotEligible
		}
		return err
	})
}

// Secrets returns the secrets a device may connect with now: B while it can still pair (I, Q, G), K once it has one (G, A).
func (r *Registry) Secrets(id string) (b, k string, ok bool) {
	_ = r.locked(func() error {
		rc, found := r.recs[id]
		if !found {
			return nil
		}
		switch rc.state {
		case dsI, dsQ:
			b = rc.B
		case dsG:
			b, k = rc.B, rc.K
		case dsA:
			k = rc.K
		}
		ok = b != "" || k != ""
		return nil
	})
	return b, k, ok
}

// Secret is one valid secret and the device it belongs to (the relay needs one room for each).
type Secret struct {
	ID     string
	Secret string
	IsK    bool
}

// ActiveSecrets lists every secret that may connect now.
func (r *Registry) ActiveSecrets() []Secret {
	var out []Secret
	for _, id := range r.idsLocked() {
		b, k, _ := r.Secrets(id)
		if b != "" {
			out = append(out, Secret{ID: id, Secret: b})
		}
		if k != "" {
			out = append(out, Secret{ID: id, Secret: k, IsK: true})
		}
	}
	return out
}

func (r *Registry) idsLocked() []string {
	r.mu.Lock()
	defer r.mu.Unlock()
	return append([]string(nil), r.order...)
}

// Revoke is REVOKE.
func (r *Registry) Revoke(id string) error {
	return r.locked(func() error {
		rc, err := r.get(id)
		if err != nil {
			return err
		}
		out, err := r.fire(rc, deRevoke, nil, false)
		if out == ignored {
			return ErrAlreadyRevoked
		}
		return err
	})
}

// Reissue is REISSUE: a new first secret; the device must pair again.
func (r *Registry) Reissue(id string) error {
	return r.locked(func() error {
		rc, err := r.get(id)
		if err != nil {
			return err
		}
		out, err := r.fire(rc, deReissue, nil, false)
		if out == ignored {
			return ErrRevoked
		}
		return err
	})
}

// Forget is FORGET: delete a revoked or expired record.
func (r *Registry) Forget(id string) error {
	return r.locked(func() error {
		rc, err := r.get(id)
		if err != nil {
			return err
		}
		out, err := r.fire(rc, deForget, nil, false)
		if out == ignored {
			return ErrStillValid
		}
		return err
	})
}

// Repack is REPACK: what a new .snplg for the device must carry. It changes nothing.
func (r *Registry) Repack(id string) (PackageData, error) {
	var out PackageData
	err := r.locked(func() error {
		rc, err := r.get(id)
		if err != nil {
			return err
		}
		switch o, err := r.fire(rc, deRepack, &out, false); {
		case err != nil:
			return err
		case o == ignored && rc.state == dsR:
			return ErrRevoked
		case o == ignored:
			return ErrExpired
		}
		return nil
	})
	return out, err
}

// Tick fires the timers that are due (WINDOW, ACK_TIMEOUT, TTL). The caller runs it about once a second.
func (r *Registry) Tick() {
	_ = r.locked(func() error {
		now := r.now()
		for _, id := range r.order { // a timer never adds or removes a record
			rc := r.recs[id]
			if rc.state == dsQ && !rc.windowEnd.IsZero() && !now.Before(rc.windowEnd) {
				_, _ = r.fire(rc, deWindow, nil, false)
			}
			if rc.state == dsG && !rc.ackEnd.IsZero() && !now.Before(rc.ackEnd) {
				_, _ = r.fire(rc, deAckTimeout, nil, false)
			}
			if (rc.state == dsI || rc.state == dsQ || rc.state == dsG) && !now.Before(r.deadline(rc)) {
				_, _ = r.fire(rc, deTtl, nil, false)
			}
		}
		return nil
	})
}

// Info is a record as the owner sees it (never a secret).
type Info struct {
	ID       string
	Name     string
	Gen      int
	Status   string // issued, waiting, granting, active, expired, revoked
	Waiting  int    // requests waiting for the owner's code
	IssuedAt time.Time
	Expires  time.Time // when B stops being valid (issued, waiting and granting only)
	LastSeen time.Time
}

var statusName = map[devState]string{dsI: "issued", dsQ: "waiting", dsG: "granting", dsA: "active", dsX: "expired", dsR: "revoked"}

// List returns every record in the order they were added.
func (r *Registry) List() []Info {
	r.mu.Lock()
	defer r.mu.Unlock()
	out := make([]Info, 0, len(r.order))
	for _, id := range r.order {
		rc := r.recs[id]
		in := Info{ID: rc.ID, Name: rc.Name, Gen: rc.Gen, Status: statusName[rc.state], Waiting: len(rc.pending), IssuedAt: rc.IssuedAt, LastSeen: rc.LastSeen}
		if rc.B != "" {
			in.Expires = r.deadline(rc)
		}
		out = append(out, in)
	}
	return out
}

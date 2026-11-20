package devices

import (
	"errors"
	"io"
	"log"
	"strings"
	"sync"
	"testing"
	"time"
)

type fakeClock struct {
	mu sync.Mutex
	t  time.Time
}

func (c *fakeClock) Now() time.Time {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.t
}
func (c *fakeClock) Advance(d time.Duration) {
	c.mu.Lock()
	c.t = c.t.Add(d)
	c.mu.Unlock()
}

// memStore is a Store in memory that can be made to fail.
type memStore struct {
	data     []byte
	saves    int
	loadErr  error
	saveErr  error
	onSave   func([]byte) // called with the bytes at the moment of a successful save
	saveLeft int          // when > 0, the next saveLeft saves succeed and then every save fails
}

func (m *memStore) Load() ([]byte, error) { return m.data, m.loadErr }
func (m *memStore) Save(b []byte) error {
	if m.saveErr != nil {
		return m.saveErr
	}
	m.saves++
	m.data = append([]byte(nil), b...)
	if m.onSave != nil {
		m.onSave(m.data)
	}
	return nil
}

// fakeConn records what the ledger tells a device.
type fakeConn struct {
	mu       sync.Mutex
	grants   []grant
	denied   []string
	grantErr error
	// atGrant captures the ledger's bytes when K is sent: it must already be stored.
	store   *memStore
	atGrant []byte
}

type grant struct {
	key string
	eps []string
}

func (c *fakeConn) Grant(key string, eps []string) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.grants = append(c.grants, grant{key, eps})
	if c.store != nil {
		c.atGrant = append([]byte(nil), c.store.data...)
	}
	return c.grantErr
}

func (c *fakeConn) Deny(reason string) {
	c.mu.Lock()
	c.denied = append(c.denied, reason)
	c.mu.Unlock()
}

func (c *fakeConn) reasons() string { return strings.Join(c.denied, ",") }

// counterRand yields distinct bytes (0,1,2,…) so every secret and id differs, deterministically.
type counterRand struct{ n byte }

func (r *counterRand) Read(p []byte) (int, error) {
	for i := range p {
		p[i] = r.n
		r.n++
	}
	return len(p), nil
}

// scriptRand serves the queued chunks first, then continues with a counter.
type scriptRand struct {
	queue [][]byte
	rest  counterRand
}

func (r *scriptRand) Read(p []byte) (int, error) {
	if len(r.queue) > 0 {
		chunk := r.queue[0]
		r.queue = r.queue[1:]
		copy(p, chunk)
		return len(p), nil
	}
	return r.rest.Read(p)
}

type failRand struct{}

func (failRand) Read([]byte) (int, error) { return 0, errors.New("no randomness") }

type hookLog struct {
	mu        sync.Mutex
	changed   int
	closed    []string
	endpoints [][2]string
}

type harness struct {
	t     *testing.T
	reg   *Registry
	clock *fakeClock
	store *memStore
	hooks *hookLog
	logs  *strings.Builder
}

type logWriter struct {
	mu sync.Mutex
	b  *strings.Builder
}

func (w *logWriter) Write(p []byte) (int, error) {
	w.mu.Lock()
	defer w.mu.Unlock()
	return w.b.Write(p)
}

func newHarness(t *testing.T, mutate ...func(*Config)) *harness {
	t.Helper()
	h := &harness{t: t, clock: &fakeClock{t: time.Date(2026, 11, 20, 9, 0, 0, 0, time.UTC)}, store: &memStore{}, hooks: &hookLog{}, logs: &strings.Builder{}}
	cfg := Config{Now: h.clock.Now, Rand: &counterRand{}, Log: log.New(&logWriter{b: h.logs}, "", 0)}
	for _, m := range mutate {
		m(&cfg)
	}
	h.reg = h.mustNew(cfg)
	return h
}

func (h *harness) hooksFor() Hooks {
	return Hooks{
		Endpoints: func(id, k string) []string {
			h.hooks.mu.Lock()
			defer h.hooks.mu.Unlock()
			h.hooks.endpoints = append(h.hooks.endpoints, [2]string{id, k})
			return []string{"wss://bridge.example/v1?d=" + id}
		},
		Changed:  func() { h.hooks.mu.Lock(); h.hooks.changed++; h.hooks.mu.Unlock() },
		CloseAll: func(id string) { h.hooks.mu.Lock(); h.hooks.closed = append(h.hooks.closed, id); h.hooks.mu.Unlock() },
	}
}

func (h *harness) mustNew(cfg Config) *Registry {
	h.t.Helper()
	r, skipped, err := New(cfg, h.store, h.hooksFor())
	if err != nil {
		h.t.Fatalf("New: %v", err)
	}
	if len(skipped) != 0 {
		h.t.Fatalf("New skipped records: %v", skipped)
	}
	return r
}

// reload builds a new registry on the same store, as a restart does.
func (h *harness) reload() *Registry {
	h.t.Helper()
	cfg := h.reg.cfg
	return h.mustNew(cfg)
}

func (h *harness) add(name string) (id string) {
	h.t.Helper()
	id, err := h.reg.Add(name)
	if err != nil {
		h.t.Fatalf("Add(%q): %v", name, err)
	}
	return id
}

func (h *harness) gen(id string) int {
	h.t.Helper()
	for _, in := range h.reg.List() {
		if in.ID == id {
			return in.Gen
		}
	}
	h.t.Fatalf("no device %s", id)
	return 0
}

func (h *harness) request(id, code string) *fakeConn {
	h.t.Helper()
	c := &fakeConn{store: h.store}
	if err := h.reg.Request(id, h.gen(id), code, c); err != nil {
		h.t.Fatalf("Request: %v", err)
	}
	return c
}

func (h *harness) status(id string) string {
	h.t.Helper()
	for _, in := range h.reg.List() {
		if in.ID == id {
			return in.Status
		}
	}
	return "gone"
}

func (h *harness) mustStatus(id, want string) {
	h.t.Helper()
	if got := h.status(id); got != want {
		h.t.Fatalf("status of %s = %s, want %s", id, got, want)
	}
}

// pair takes a new device through request → confirm and returns it granted (state G).
func (h *harness) granted(name, code string) (id string, c *fakeConn) {
	h.t.Helper()
	id = h.add(name)
	c = h.request(id, code)
	if got, err := h.reg.Confirm(code); err != nil || got != name {
		h.t.Fatalf("Confirm = %q, %v", got, err)
	}
	return id, c
}

// active takes a new device all the way to A.
func (h *harness) active(name, code string) (id string, c *fakeConn) {
	h.t.Helper()
	id, c = h.granted(name, code)
	if err := h.reg.Ack(id, h.gen(id)); err != nil {
		h.t.Fatalf("Ack: %v", err)
	}
	return id, c
}

// ledger decodes what the store holds now.
func (h *harness) ledger() []Record {
	h.t.Helper()
	recs, skipped, err := decode(h.store.data)
	if err != nil || len(skipped) != 0 {
		h.t.Fatalf("stored ledger unusable: %v %v", err, skipped)
	}
	return recs
}

func discard() io.Writer { return io.Discard }

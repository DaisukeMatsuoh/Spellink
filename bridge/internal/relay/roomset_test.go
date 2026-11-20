package relay

import (
	"context"
	"io"
	"log"
	"sync"
	"testing"
	"time"
)

type runLog struct {
	mu      sync.Mutex
	started []string
	stopped []string
}

func (l *runLog) run(ctx context.Context, base, room string, serve ServeFunc, logger *log.Logger) {
	l.mu.Lock()
	l.started = append(l.started, room)
	l.mu.Unlock()
	<-ctx.Done()
	l.mu.Lock()
	l.stopped = append(l.stopped, room)
	l.mu.Unlock()
}

func (l *runLog) snapshot() (started, stopped int) {
	l.mu.Lock()
	defer l.mu.Unlock()
	return len(l.started), len(l.stopped)
}

func eventually(t *testing.T, what string, f func() bool) {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		if f() {
			return
		}
		time.Sleep(5 * time.Millisecond)
	}
	t.Fatalf("timed out waiting for %s", what)
}

func newSet(t *testing.T) (*RoomSet, *runLog, context.CancelFunc) {
	ctx, cancel := context.WithCancel(context.Background())
	rs := NewRoomSet(ctx, "wss://relay.example", log.New(io.Discard, "", 0))
	l := &runLog{}
	rs.run = l.run
	return rs, l, cancel
}

func TestRoomSetFollowsTheWantedRooms(t *testing.T) {
	rs, l, cancel := newSet(t)
	defer cancel()
	f := ServeFunc(nil)
	rs.Sync(map[string]ServeFunc{"a": f, "b": f})
	eventually(t, "two rooms", func() bool { s, _ := l.snapshot(); return s == 2 })
	if rs.Len() != 2 {
		t.Fatalf("Len = %d", rs.Len())
	}
	rs.Sync(map[string]ServeFunc{"a": f, "c": f}) // b leaves, c arrives, a stays untouched
	eventually(t, "b stopped and c started", func() bool { s, p := l.snapshot(); return s == 3 && p == 1 })
	rs.Sync(map[string]ServeFunc{"a": f, "c": f}) // no change
	rs.Sync(nil)
	eventually(t, "all stopped", func() bool { _, p := l.snapshot(); return p == 3 })
	if rs.Len() != 0 {
		t.Errorf("Len = %d", rs.Len())
	}
	l.mu.Lock()
	defer l.mu.Unlock()
	if len(l.started) != 3 {
		t.Errorf("an unchanged room was registered again: %v", l.started)
	}
}

func TestRoomSetStopsEverythingWithItsContext(t *testing.T) {
	rs, l, cancel := newSet(t)
	rs.Sync(map[string]ServeFunc{"a": nil})
	eventually(t, "started", func() bool { s, _ := l.snapshot(); return s == 1 })
	cancel()
	eventually(t, "stopped by the parent context", func() bool { _, p := l.snapshot(); return p == 1 })
}

func TestConnectRoomIsConnectForAKnownRoom(t *testing.T) {
	// the default runner is ConnectRoom; it must stop at once when its context is already over
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	done := make(chan struct{})
	go func() {
		NewRoomSet(ctx, "wss://relay.example", log.New(io.Discard, "", 0)).run(ctx, "wss://relay.example", RoomID("x"), nil, log.New(io.Discard, "", 0))
		close(done)
	}()
	select {
	case <-done:
	case <-time.After(2 * time.Second):
		t.Fatal("ConnectRoom did not stop")
	}
}

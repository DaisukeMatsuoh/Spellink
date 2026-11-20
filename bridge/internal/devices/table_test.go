package devices

import (
	"reflect"
	"strings"
	"testing"
)

// The table in docs/spec/pairing.md is generated into devTable. These tests check that the code
// around it keeps its side of the bargain: every cell exists, every action has a handler, and
// each cell does what it says on a record that really is in that state.

func TestTableIsCompleteAndEveryActionHasAHandler(t *testing.T) {
	for _, s := range devStates {
		for _, e := range devEvents {
			if _, ok := devTable[s][e]; !ok {
				t.Errorf("no cell for %s × %s", s, e)
			}
		}
	}
	for _, a := range devActions {
		if actions[a] == nil {
			t.Errorf("action %s has no handler", a)
		}
	}
	if len(actions) != len(devActions) {
		t.Errorf("%d handlers for %d actions in the table", len(actions), len(devActions))
	}
}

// recordIn builds a record that is in state s by doing what a real device does.
func (h *harness) recordIn(s devState) *rec {
	h.t.Helper()
	switch s {
	case dsNoRecord:
		return &rec{state: dsNoRecord}
	case dsIssued:
		return h.reg.recs[h.add("dev-I")]
	case dsRequested:
		id := h.add("dev-Q")
		h.request(id, "11111111")
		return h.reg.recs[id]
	case dsGranted:
		id, _ := h.granted("dev-G", "11111111")
		return h.reg.recs[id]
	case dsActive:
		id, _ := h.active("dev-A", "11111111")
		return h.reg.recs[id]
	case dsExpired:
		id := h.add("dev-X")
		h.clock.Advance(h.reg.cfg.TTL)
		h.reg.Tick()
		return h.reg.recs[id]
	default: // dsRevoked
		id := h.add("dev-R")
		if err := h.reg.Revoke(id); err != nil {
			h.t.Fatal(err)
		}
		return h.reg.recs[id]
	}
}

// argFor is what the caller of the event hands to the table.
func argFor(rc *rec, e devEvent) any {
	switch e {
	case deOwnerAdds:
		return "another-name"
	case deDeviceRequests, deRequestDrops, deOwnerConfirms:
		var c Conn = &fakeConn{}
		if len(rc.pending) > 0 {
			c = rc.pending[0].conn
		}
		switch e {
		case deDeviceRequests:
			return &request{code: "22222222", conn: &fakeConn{}}
		case deRequestDrops:
			return c
		default:
			if len(rc.pending) > 0 {
				return rc.pending[0]
			}
			return &pending{code: "99999999", conn: c}
		}
	case deOwnerRepacks:
		return &PackageData{}
	}
	return nil
}

func TestEveryCellDoesWhatTheTableSays(t *testing.T) {
	for _, s := range devStates {
		for _, e := range devEvents {
			cell := devTable[s][e]
			branches := []bool{false}
			if cell.kind == cellGuard {
				branches = []bool{false, true}
			}
			for _, spent := range branches {
				h := newHarness(t)
				rc := h.recordIn(s)
				before := rc.Record
				beforeState, beforePending := rc.state, len(rc.pending)
				var out outcome
				var err error
				_ = h.reg.locked(func() error {
					out, err = h.reg.fire(rc, e, argFor(rc, e), spent)
					return nil
				})
				name := string(s) + " × " + string(e)
				if err != nil {
					t.Errorf("%s (spent=%v): %v", name, spent, err)
					continue
				}
				switch cell.kind {
				case cellNever:
					if out != never {
						t.Errorf("%s: outcome %v, want never", name, out)
					}
				case cellIgnore:
					if out != ignored {
						t.Errorf("%s: outcome %v, want ignored", name, out)
					}
				default:
					if out != moved {
						t.Errorf("%s: outcome %v, want moved", name, out)
					}
				}
				if cell.kind == cellNever || cell.kind == cellIgnore {
					if rc.state != beforeState || !reflect.DeepEqual(rc.Record, before) || len(rc.pending) != beforePending {
						t.Errorf("%s: a cell that does nothing changed the record", name)
					}
					continue
				}
				want := cell.next
				if cell.kind == cellGuard {
					want = cell.more.next
					if spent {
						want = cell.spent.next
					}
				}
				// ISSUE on N registers the new record; everything else keeps its identity.
				if rc.state != want {
					t.Errorf("%s (spent=%v): state %s, want %s", name, spent, rc.state, want)
				}
			}
		}
	}
}

func TestUnknownCellIsRecordedNotFatal(t *testing.T) {
	h := newHarness(t)
	rc := &rec{state: "?"}
	_ = h.reg.locked(func() error {
		if out, err := h.reg.fire(rc, deTtlEnds, nil, false); out != never || err != nil {
			t.Errorf("fire on an unknown state = %v, %v", out, err)
		}
		return nil
	})
	if !strings.Contains(h.logs.String(), "devices.unknown") {
		t.Errorf("no record of the unknown cell: %q", h.logs.String())
	}
}

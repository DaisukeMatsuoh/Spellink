package server

import (
	"fmt"
	"os"
	"strings"
	"testing"
)

// render writes a cell in the notation of docs/spec/bridge-run.md.
func render(state runState, c runCell) string {
	switch c.kind {
	case cellNever:
		return "—" + c.note
	case cellIgnore:
		if c.action != "" {
			return "×" + string(c.action)
		}
		return "×" + c.note
	case cellDo:
		if c.next == state {
			return string(c.action)
		}
		return fmt.Sprintf("%s→%s", c.action, c.next)
	default:
		return fmt.Sprintf("%s→%s [more] / %s→%s [spent]", c.more.action, c.more.next, c.spent.action, c.spent.next)
	}
}

// The document is the source of the table; the generated code must say the same in every cell.
// If this fails: change the document, then run `npm run spec` in plugin/.
func TestTheTableInTheDocumentIsTheTableInTheCode(t *testing.T) {
	raw, err := os.ReadFile("../../../docs/spec/bridge-run.md")
	if err != nil {
		t.Fatal(err)
	}
	var header []runEvent
	seen := 0
	for _, line := range strings.Split(string(raw), "\n") {
		if !strings.HasPrefix(line, "|") {
			continue
		}
		cells := strings.Split(strings.Trim(line, "| "), "|")
		for i := range cells {
			cells[i] = strings.TrimSpace(cells[i])
		}
		if strings.HasPrefix(cells[0], "状態＼イベント") {
			header = nil
			for _, e := range cells[1:] {
				header = append(header, runEvent(e))
			}
			continue
		}
		if !strings.HasPrefix(cells[0], "**") || header == nil {
			continue
		}
		state := runState(strings.Trim(cells[0], "*"))
		for i, e := range header {
			code, ok := runTable[state][e]
			if !ok {
				t.Errorf("%s × %s is in the document but not in the code", state, e)
				continue
			}
			if got := render(state, code); got != cells[i+1] {
				t.Errorf("%s × %s: the document says %q, the code says %q", state, e, cells[i+1], got)
			}
			seen++
		}
	}
	if want := len(runStates) * len(runEvents); seen != want {
		t.Fatalf("the document has %d cells, the table has %d", seen, want)
	}
}

// Every cell does what the table says, with the real machine and recording handlers.
func TestEveryCellDoesWhatTheTableSays(t *testing.T) {
	for _, st := range runStates {
		for _, ev := range runEvents {
			cell := runTable[st][ev]
			var called []runAction
			var logs []string
			handlers := map[runAction]func(any){}
			for _, a := range runActions {
				a := a
				handlers[a] = func(any) { called = append(called, a) }
			}
			m := runMachine{state: st, handlers: handlers, log: func(f string, a ...any) { logs = append(logs, fmt.Sprintf(f, a...)) }}
			m.dispatch(ev, nil, false)
			at := fmt.Sprintf("%s × %s", st, ev)
			switch cell.kind {
			case cellNever:
				if len(called) != 0 || m.state != st || len(logs) == 0 || !strings.HasPrefix(logs[0], "run.never") {
					t.Errorf("%s: a never cell must do nothing, stay, and say so (called %v, state %s, logs %v)", at, called, m.state, logs)
				}
			case cellIgnore:
				want := 0
				if cell.action != "" {
					want = 1
				}
				if m.state != st || len(called) != want {
					t.Errorf("%s: an ignore cell stays and calls %d action(s), called %v, state %s", at, want, called, m.state)
				}
			case cellDo:
				if m.state != cell.next || len(called) != 1 || called[0] != cell.action {
					t.Errorf("%s: want %s→%s, got %v→%s", at, cell.action, cell.next, called, m.state)
				}
			case cellGuard:
				if m.state != cell.more.next || len(called) != 1 || called[0] != cell.more.action {
					t.Errorf("%s (more): want %s→%s, got %v→%s", at, cell.more.action, cell.more.next, called, m.state)
				}
			}
		}
	}
}

func TestTheShapeOfTheTable(t *testing.T) {
	next := func(st runState) (out []runState) {
		for _, ev := range runEvents {
			c := runTable[st][ev]
			switch c.kind {
			case cellDo:
				out = append(out, c.next)
			case cellGuard:
				out = append(out, c.more.next, c.spent.next)
			}
		}
		return
	}
	// a run is created only from N, and begins working
	if c := runTable[rsNoRun][reAsk]; c.kind != cellDo || c.next != rsAskingLive {
		t.Errorf("N × ASK must create a working run, got %+v", c)
	}
	// a finished run never goes back to work: its only ways on are to the other finished state or to N
	for _, st := range []runState{rsDoneLive, rsDoneAway} {
		for _, n := range next(st) {
			if n == rsAskingLive || n == rsAskingAway {
				t.Errorf("%s must not go back to work (%s)", st, n)
			}
		}
	}
	// everything that is working can end, and the only way into N from a run is the sweep
	reach := func(from runState) map[runState]bool {
		seen := map[runState]bool{from: true}
		todo := []runState{from}
		for len(todo) > 0 {
			s := todo[len(todo)-1]
			todo = todo[:len(todo)-1]
			for _, n := range next(s) {
				if !seen[n] {
					seen[n] = true
					todo = append(todo, n)
				}
			}
		}
		return seen
	}
	for _, st := range []runState{rsAskingLive, rsAskingAway} {
		if r := reach(st); !r[rsDoneLive] || !r[rsDoneAway] {
			t.Errorf("%s must be able to finish: %v", st, r)
		}
	}
	for _, st := range runStates {
		for _, ev := range runEvents {
			c := runTable[st][ev]
			for _, n := range []runState{c.next, c.more.next, c.spent.next} {
				if n == rsNoRun && st != rsNoRun && ev != reSwept {
					t.Errorf("%s × %s leads to N: only the sweep forgets a run", st, ev)
				}
			}
		}
	}
	// both ways of ending an answer in the middle are covered in every working state
	for _, st := range []runState{rsAskingLive, rsAskingAway} {
		for _, ev := range []runEvent{reCancel, reAiFails, reAiGoesSilent, reAiFinishes, rePagesRunOut} {
			if c := runTable[st][ev]; c.kind != cellDo || (c.next != rsDoneLive && c.next != rsDoneAway) {
				t.Errorf("%s × %s must finish the run, got %+v", st, ev, c)
			}
		}
	}
}

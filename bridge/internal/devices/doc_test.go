package devices

import (
	"fmt"
	"os"
	"strings"
	"testing"
)

// render writes a cell in the notation of docs/spec/pairing.md.
func render(state devState, c devCell) string {
	switch c.kind {
	case cellNever:
		return "—" + c.note
	case cellIgnore:
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
	raw, err := os.ReadFile("../../../docs/spec/pairing.md")
	if err != nil {
		t.Fatal(err)
	}
	in := false
	var header []devEvent
	seen := 0
	for _, line := range strings.Split(string(raw), "\n") {
		if strings.HasPrefix(line, "## ") {
			in = strings.TrimSpace(line[3:]) == "記録: 状態遷移表"
			continue
		}
		if !in || !strings.HasPrefix(line, "|") {
			continue
		}
		cells := strings.Split(strings.Trim(line, "| "), "|")
		for i := range cells {
			cells[i] = strings.TrimSpace(cells[i])
		}
		if strings.HasPrefix(cells[0], "状態＼イベント") {
			header = nil
			for _, e := range cells[1:] {
				header = append(header, devEvent(e))
			}
			continue
		}
		if !strings.HasPrefix(cells[0], "**") || header == nil {
			continue
		}
		state := devState(strings.Trim(cells[0], "*"))
		for i, e := range header {
			code, ok := devTable[state][e]
			if !ok {
				t.Errorf("%s × %s is in the document but not in the code", state, e)
				continue
			}
			got := render(state, code)
			// a cell that stays in its state may be written with or without the arrow to itself
			if code.kind == cellDo && code.next == state && cells[i+1] == fmt.Sprintf("%s→%s", code.action, state) {
				got = cells[i+1]
			}
			if got != cells[i+1] {
				t.Errorf("%s × %s: the document says %q, the code says %q", state, e, cells[i+1], got)
			}
			seen++
		}
	}
	if want := len(devStates) * len(devEvents); seen != want {
		t.Fatalf("the document has %d cells, the table has %d", seen, want)
	}
}

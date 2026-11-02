package ink

import (
	"bytes"
	"encoding/json"
	"reflect"
	"strings"
	"testing"
)

// Tiny synthetic font: every glyph is one vertical stroke; full-width glyphs are 1 em wide.
func testFont() *Font {
	f := &Font{Em: 100, Glyphs: map[rune]Glyph{}}
	for _, r := range "あいうえお、。漢字abcdefghij" {
		adv := int16(100)
		if r >= 'a' && r <= 'j' {
			adv = 50
		}
		f.Glyphs[r] = Glyph{Advance: adv, Strokes: [][]int16{{50, 0, 50, 100}}}
	}
	return f
}

var testLayout = Layout{X: 0, Y: 0, Width: 500, PageHeight: 1000, BottomMargin: 0, CharSize: 100}

func compose(text string, l Layout) ([]Line, *Composer) {
	c := NewComposer(testFont(), l, 1)
	return append(c.Push(text), c.Finish()...), c
}

func texts(lines []Line) []string {
	out := []string{}
	for _, l := range lines {
		out = append(out, l.Text)
	}
	return out
}

func eq(t *testing.T, got, want any) {
	t.Helper()
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("got %#v, want %#v", got, want)
	}
}

func TestWrapsJapanese(t *testing.T) {
	lines, _ := compose("あいうえおあいう", testLayout)
	eq(t, texts(lines), []string{"あいうえお", "あいう"})
	eq(t, []int{lines[0].Index, lines[1].Index}, []int{0, 1})
}

func TestStreamsCompleteLinesOnly(t *testing.T) {
	c := NewComposer(testFont(), testLayout, 1)
	eq(t, len(c.Push("あいうえ")), 0)
	eq(t, len(c.Push("お")), 0) // fits exactly; may still be followed by punctuation
	eq(t, texts(c.Push("あ")), []string{"あいうえお"})
	eq(t, texts(c.Finish()), []string{"あ"})
}

func TestPunctuationNeverStartsLine(t *testing.T) {
	lines, _ := compose("あいうえお。あ", testLayout)
	eq(t, texts(lines), []string{"あいうえお。", "あ"})
}

func TestLatinWordsWrapWhole(t *testing.T) {
	lines, _ := compose("abc abcdefg hij", testLayout) // 10 half-width chars per line
	eq(t, texts(lines), []string{"abc ", "abcdefg ", "hij"})
}

func minY(l Line) int {
	m := 1 << 30
	for i := 1; i < len(l.Strokes[0].P); i += 2 {
		m = min(m, l.Strokes[0].P[i])
	}
	return m
}

func TestParagraphGapAndTruncation(t *testing.T) {
	l := testLayout
	l.PageHeight = 500
	lines, c := compose("あ\n\nい\nう\nえ", l)
	// Line pitch 145px: あ at 0, い at 217.5 after the paragraph gap, う at 362.5; え would end past 500.
	eq(t, texts(lines), []string{"あ", "い", "う"})
	if y := minY(lines[1]); y < 190 || y > 245 {
		t.Fatalf("paragraph gap: い at %d", y)
	}
	eq(t, c.Truncated, true)
}

func TestContinuesOnNextPage(t *testing.T) {
	l := testLayout
	l.PageHeight, l.Y, l.MaxPages, l.ContinueTop = 500, 200, 2, 10
	lines, c := compose("あ\nい\nう\nえ\nお", l)
	// Page 0 holds lines at 200 and 345; the rest continue from y=10 on page 1.
	eq(t, texts(lines), []string{"あ", "い", "う", "え", "お"})
	eq(t, []int{lines[0].Page, lines[1].Page, lines[2].Page, lines[4].Page}, []int{0, 0, 1, 1})
	if y := minY(lines[2]); y > 40 {
		t.Fatalf("continuation should start near ContinueTop, got %d", y)
	}
	eq(t, c.Pages, 2)
	eq(t, c.Truncated, false)
}

func TestMissingGlyphsReported(t *testing.T) {
	lines, c := compose("あ🙂い", testLayout)
	eq(t, c.Missing, map[rune]bool{'🙂': true})
	eq(t, len(lines[0].Strokes), 2)
}

func TestMarkdownDropped(t *testing.T) {
	lines, _ := compose("**あ**`い`", testLayout)
	eq(t, texts(lines), []string{"あい"})
}

func TestResample(t *testing.T) {
	eq(t, Resample([]float64{0, 0, 10, 0}, 3), []float64{0, 0, 3, 0, 6, 0, 9, 0, 10, 0})
}

func TestDeterministic(t *testing.T) {
	a := NewComposer(testFont(), testLayout, 42).Push("あいう\n")
	b := NewComposer(testFont(), testLayout, 42).Push("あいう\n")
	eq(t, a, b)
}

func TestCapacity(t *testing.T) {
	// 5 chars per line × floor(1000/145)=6 lines.
	eq(t, Capacity(testLayout), 30)
}

func TestFontRoundTrip(t *testing.T) {
	f := testFont()
	f.Sources = []string{"test"}
	var buf bytes.Buffer
	if err := f.Write(&buf); err != nil {
		t.Fatal(err)
	}
	g, err := ReadFont(&buf)
	if err != nil {
		t.Fatal(err)
	}
	eq(t, g, f)
	if _, err := ReadFont(strings.NewReader("nope")); err == nil {
		t.Fatal("expected error for bad magic")
	}
}

func TestSlopeTiltsTheWholeBlockUniformly(t *testing.T) {
	flat, _ := compose("ああああああああああ", testLayout)
	l := testLayout
	l.Slope = -0.02 // rises to the right
	tilted, _ := compose("ああああああああああ", l)
	shift := func(line int, stroke int) int {
		return tilted[line].Strokes[stroke].P[1] - flat[line].Strokes[stroke].P[1]
	}
	// Same position in both lines → same shift (lines stay parallel); further right → higher.
	if shift(0, 4) != shift(1, 4) {
		t.Fatalf("lines should share one slope: %d vs %d", shift(0, 4), shift(1, 4))
	}
	if !(shift(0, 4) < shift(0, 0)) {
		t.Fatalf("right end should rise: start %d, end %d", shift(0, 0), shift(0, 4))
	}
}

func TestFragmentsStartEarlyAndJoinSeamlessly(t *testing.T) {
	l := testLayout
	l.Width = 1000 // 10 full-width chars per line
	c := NewComposer(testFont(), l, 1)
	early := c.Push("あいうえおあいうえ") // 9 chars: line not full yet
	if len(early) == 0 || !early[0].Cont {
		t.Fatalf("expected an early fragment, got %v", early)
	}
	rest := append(c.Push("お"), c.Finish()...)
	all := append(early, rest...)
	var text strings.Builder
	for _, ln := range all {
		text.WriteString(ln.Text)
	}
	if text.String() != "あいうえおあいうえお" {
		t.Fatalf("fragments must add up to the line, got %q", text.String())
	}
	// Fragments of one line share a baseline and continue to the right.
	firstX := all[0].Strokes[0].P[0]
	lastX := all[len(all)-1].Strokes[0].P[0]
	if lastX <= firstX {
		t.Fatalf("later fragments must start further right: %d <= %d", lastX, firstX)
	}
}

func TestFragmentWithoutGlyphsHasEmptyStrokesNotNull(t *testing.T) {
	// Regression: "🙂" has no glyph, so a fragment made of it carried `"strokes":null`, which
	// crashed the plugin with "Cannot read property 'length' of null".
	c := NewComposer(testFont(), testLayout, 1)
	out := append(c.Push("🙂🙂🙂🙂🙂🙂🙂🙂🙂"), c.Finish()...)
	if len(out) == 0 {
		t.Fatal("expected a fragment")
	}
	b, err := json.Marshal(out)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(b), `"strokes":null`) {
		t.Fatalf("strokes must be [] not null: %s", b)
	}
}

func TestStartLimitSendsTextToTheNextPageWhenThereIsNoRoom(t *testing.T) {
	l := testLayout
	l.Y, l.StartLimit, l.MaxPages, l.ContinueTop = 200, 250, 2, 10 // < one line of room
	lines, _ := compose("あ", l)
	if lines[0].Page != 1 {
		t.Fatalf("no room above StartLimit: want page 1, got %d", lines[0].Page)
	}
}

func TestStartLimitKeepsRoomyAnswersOnTheFirstPage(t *testing.T) {
	l := testLayout
	l.Y, l.StartLimit, l.MaxPages, l.ContinueTop = 200, 600, 2, 10
	lines, _ := compose("あい\nう", l)
	for _, ln := range lines {
		if ln.Page != 0 {
			t.Fatalf("%q should fit on the first page", ln.Text)
		}
	}
	l.StartLimit = 360 // room for one line only
	lines, _ = compose("あい\nう", l)
	if lines[len(lines)-1].Page != 1 {
		t.Fatal("second line must continue on the next page")
	}
}

func TestCapacityHonoursStartLimit(t *testing.T) {
	l := testLayout
	l.StartLimit = 300 // two lines of 145px from y=0 → 2×5 chars
	if got := Capacity(l); got != 10 {
		t.Fatalf("got %d, want 10", got)
	}
}

func TestLetterSpacingWidensHalfAndFullWidthIndependently(t *testing.T) {
	width := func(text string, full, half float64) int {
		l := testLayout
		l.Width = 5000
		l.SpacingFull, l.SpacingHalf = full, half
		lines, _ := compose(text, l)
		last := lines[0].Strokes[len(lines[0].Strokes)-1]
		return last.P[0] // x of the last glyph
	}
	if width("あああああ", 1, 1) >= width("あああああ", 1.3, 1) {
		t.Error("full-width spacing should push later characters to the right")
	}
	if width("あああああ", 1, 1) != width("あああああ", 1, 1.5) {
		t.Error("half-width spacing must not affect full-width characters")
	}
	if width("abcde", 1, 1) >= width("abcde", 1, 1.5) {
		t.Error("half-width spacing should push later characters to the right")
	}
	if width("abcde", 1, 1) != width("abcde", 1.4, 1) {
		t.Error("full-width spacing must not affect half-width characters")
	}
}

func TestLetterSpacingKeepsTheInkCentredInItsSlot(t *testing.T) {
	l := testLayout
	l.SpacingFull = 1.4
	lines, _ := compose("あ", l)
	// The glyph is a stroke at x = 50 of 100; with 40% extra room it moves right by 20% of the size.
	if got := lines[0].Strokes[0].P[0]; got < 60 {
		t.Fatalf("ink should shift by half the extra room, got x=%d", got)
	}
}

func TestLetterSpacingWrapsEarlier(t *testing.T) {
	l := testLayout
	l.SpacingFull = 1.5 // 5 chars of 100px no longer fit in 500px
	lines, _ := compose("ああああ", l)
	if len(lines) < 2 {
		t.Fatalf("wider spacing should wrap sooner, got %v", texts(lines))
	}
}

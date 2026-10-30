package ink

import (
	"bytes"
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

package glyphbuild

import (
	"testing"

	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/ink"
)

const kvgSample = `<kanjivg>
<kanji id="kvg:kanji_00041"><g><path id="a" d="M54.46,14.41c-8.67,19.8-31.28,70.81-34.46,78.47"/>
<path id="b" d="M32.18,66.14c12.28,0,33.91,0,44.02,0"/></g></kanji>
<kanji id="kvg:kanji_00031"><g><path id="c" d="M45,15 L45,90"/></g></kanji>
<kanji id="kvg:kanji_03042"><g><path id="d" d="M30,30 L80,30 M55,15 L55,95"/></g></kanji>
<kanji id="kvg:kanji_30a2"><g><path id="e" d="M30,30 L80,30"/></g></kanji>
<kanji id="kvg:kanji_65e5"><g><path id="f" d="M30,20 L80,20 L80,95 L30,95 Z"/></g></kanji>
</kanjivg>`

func buildSample(t *testing.T) *ink.Font {
	t.Helper()
	f := &ink.Font{Em: em, Glyphs: map[rune]ink.Glyph{}}
	if err := addKanjiVG(f, []byte(kvgSample)); err != nil {
		t.Fatal(err)
	}
	return f
}

func glyphHeight(g ink.Glyph) int16 {
	lo, hi := int16(32767), int16(-32768)
	for _, s := range g.Strokes {
		for i := 1; i < len(s); i += 2 {
			lo, hi = min(lo, s[i]), max(hi, s[i])
		}
	}
	return hi - lo
}

func TestKanjiVGParsing(t *testing.T) {
	f := buildSample(t)
	for _, r := range []rune{'A', '1', 'あ', 'ア', '日'} {
		if _, ok := f.Glyphs[r]; !ok {
			t.Errorf("missing glyph %q", r)
		}
	}
	if n := len(f.Glyphs['A'].Strokes); n != 2 {
		t.Errorf("A should keep its 2 strokes in order, got %d", n)
	}
}

func TestKanaAreSmallerThanKanjiOnTheSameBottomLine(t *testing.T) {
	f := buildSample(t)
	kanji, hira := f.Glyphs['日'], f.Glyphs['あ']
	if glyphHeight(hira) >= glyphHeight(kanji) {
		t.Fatalf("hiragana (%d) should be shorter than kanji (%d)", glyphHeight(hira), glyphHeight(kanji))
	}
}

func TestDigitsHaveAMinimumWidthAndLatinGetsBearing(t *testing.T) {
	f := buildSample(t)
	if adv := f.Glyphs['1'].Advance; adv < int16(minDigitWidth) {
		t.Fatalf("a narrow '1' must keep a minimum pitch, got %d", adv)
	}
	if f.Glyphs['A'].Advance <= 0 {
		t.Fatal("A needs an advance")
	}
}

func TestHersheyAddsSymbolsWithoutOverridingKanjiVG(t *testing.T) {
	f := buildSample(t)
	before := f.Glyphs['A']
	js := `{"futural":{"chars":[{"d":"M5,1 L5,15 M5,20 L4,21"},{"d":"M9,1 L1,22"}]}}`
	if err := addHershey(f, []byte(js)); err != nil {
		t.Fatal(err)
	}
	if _, ok := f.Glyphs['!']; !ok {
		t.Error("Hershey '!' (U+0021) should have been added")
	}
	if len(f.Glyphs['A'].Strokes) != len(before.Strokes) {
		t.Error("existing glyphs must win over Hershey")
	}
	if err := addHershey(f, []byte("not json")); err == nil {
		t.Error("bad JSON must be an error")
	}
}

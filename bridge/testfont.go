package main

import (
	"fmt"
	"os"

	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/ink"
)

// testFont writes a tiny glyph file — every hiragana, digit and ASCII letter as a two-stroke
// mark — so that tests can run a bridge without downloading the real glyph data. It is for the
// plugin's integration tests (plugin/__tests__/integration.test.ts), not for use.
func testFont(args []string) error {
	if len(args) != 1 {
		return fmt.Errorf("usage: spellink-bridge testfont OUT")
	}
	f := &ink.Font{Em: 100, Glyphs: map[rune]ink.Glyph{}}
	mark := func(r rune, full bool) {
		adv := int16(100)
		if !full {
			adv = 55
		}
		f.Glyphs[r] = ink.Glyph{Advance: adv, Strokes: [][]int16{{10, 10, 60, 10, 60, 90}, {10, 50, 50, 50}}}
	}
	for r := 'ぁ'; r <= 'ん'; r++ {
		mark(r, true)
	}
	for r := '0'; r <= '9'; r++ {
		mark(r, false)
	}
	for r := 'a'; r <= 'z'; r++ {
		mark(r, false)
		mark(r-'a'+'A', false)
	}
	for _, r := range "。、？！：:.,?!+-=" {
		mark(r, r > 0x7f)
	}
	w, err := os.Create(args[0])
	if err != nil {
		return err
	}
	defer w.Close()
	return f.Write(w)
}

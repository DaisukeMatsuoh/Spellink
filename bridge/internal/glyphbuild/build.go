// Package glyphbuild downloads KanjiVG and the Hershey "futural" font and converts them into
// centre-line glyphs in a 1000-unit em box.
//
// KanjiVG (https://kanjivg.tagaini.net) © Ulrich Apel, CC BY-SA 3.0. The generated file is a
// derivative work and stays under CC BY-SA 3.0, which is why it is built on the user's machine
// instead of being shipped. Hershey fonts via hersheytext (MIT).
package glyphbuild

import (
	"bytes"
	"compress/gzip"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"math"
	"net/http"
	"regexp"
	"strconv"

	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/ink"
)

const (
	KanjiVGURL = "https://github.com/KanjiVG/kanjivg/releases/download/r20260714/kanjivg-20260714.xml.gz"
	HersheyURL = "https://unpkg.com/hersheytext@2.0.0/hersheytext.json"

	em          = 1000
	kvgBox      = 109.0
	sideBearing = 60.0 // around proportional (non-CJK) glyphs, em units
)

var (
	kvgScale = em / kvgBox
	// KanjiVG's "A" spans y=14.41..92.88 (cap height) in its 109 box; Hershey's spans 1..22.
	kvgBaseline  = 92.88 * kvgScale
	hersheyScale = (92.88 - 14.41) * kvgScale / 21

	kanjiRe = regexp.MustCompile(`(?s)<kanji id="kvg:kanji_([0-9a-f]+)">(.*?)</kanji>`)
	pathRe  = regexp.MustCompile(`<path [^>]*\bd="([^"]+)"`)
)

// Build downloads both sources and returns the merged font.
func Build(ctx context.Context, client *http.Client) (*ink.Font, error) {
	font := &ink.Font{
		Em: em,
		Sources: []string{
			"KanjiVG r20260714 (https://kanjivg.tagaini.net) © Ulrich Apel, CC BY-SA 3.0",
			"Hershey futural via hersheytext 2.0.0 (MIT)",
		},
		Glyphs: map[rune]ink.Glyph{},
	}

	gz, err := download(ctx, client, KanjiVGURL)
	if err != nil {
		return nil, err
	}
	zr, err := gzip.NewReader(bytes.NewReader(gz))
	if err != nil {
		return nil, fmt.Errorf("KanjiVG: %w", err)
	}
	xml, err := io.ReadAll(zr)
	if err != nil {
		return nil, fmt.Errorf("KanjiVG: %w", err)
	}
	if err := addKanjiVG(font, xml); err != nil {
		return nil, err
	}

	js, err := download(ctx, client, HersheyURL)
	if err != nil {
		return nil, err
	}
	if err := addHershey(font, js); err != nil {
		return nil, err
	}
	return font, nil
}

func addKanjiVG(font *ink.Font, xml []byte) error {
	for _, m := range kanjiRe.FindAllSubmatch(xml, -1) {
		cp, err := strconv.ParseUint(string(m[1]), 16, 32)
		if err != nil {
			continue
		}
		var strokes [][]float64
		for _, p := range pathRe.FindAllSubmatch(m[2], -1) {
			lines, err := PathToPolylines(string(p[1]), kvgScale)
			if err != nil {
				return fmt.Errorf("KanjiVG U+%04X: %w", cp, err)
			}
			strokes = append(strokes, lines...)
		}
		if len(strokes) == 0 {
			continue
		}
		r := rune(cp)
		if isFullWidth(r) {
			font.Glyphs[r] = fullWidth(strokes, r)
		} else {
			font.Glyphs[r] = proportional(strokes)
		}
	}
	return nil
}

func addHershey(font *ink.Font, js []byte) error {
	var data struct {
		Futural struct {
			Chars []struct {
				D string `json:"d"`
			} `json:"chars"`
		} `json:"futural"`
	}
	if err := json.Unmarshal(js, &data); err != nil {
		return fmt.Errorf("Hershey: %w", err)
	}
	for i, c := range data.Futural.Chars {
		r := rune(33 + i)
		if _, ok := font.Glyphs[r]; ok || c.D == "" {
			continue
		}
		lines, err := PathToPolylines(c.D, 1)
		if err != nil {
			return fmt.Errorf("Hershey %q: %w", r, err)
		}
		// Hershey: baseline at y=22, cap height 21 units. Map onto KanjiVG's Latin metrics.
		for _, s := range lines {
			for j := 0; j < len(s); j += 2 {
				s[j] *= hersheyScale
				s[j+1] = kvgBaseline - (22-s[j+1])*hersheyScale
			}
		}
		font.Glyphs[r] = proportional(lines)
	}
	return nil
}

func download(ctx context.Context, client *http.Client, url string) ([]byte, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return nil, err
	}
	res, err := client.Do(req)
	if err != nil {
		return nil, err
	}
	defer res.Body.Close()
	if res.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("GET %s: HTTP %d", url, res.StatusCode)
	}
	return io.ReadAll(res.Body)
}

func isFullWidth(r rune) bool {
	return r >= 0x2e80 && !(r >= 0xff61 && r <= 0xffdc)
}

func xRange(strokes [][]float64) (minX, maxX float64) {
	minX, maxX = math.Inf(1), math.Inf(-1)
	for _, s := range strokes {
		for i := 0; i < len(s); i += 2 {
			minX, maxX = math.Min(minX, s[i]), math.Max(maxX, s[i])
		}
	}
	return
}

func shifted(strokes [][]float64, dx float64) [][]int16 {
	out := make([][]int16, len(strokes))
	for i, s := range strokes {
		out[i] = make([]int16, len(s))
		for j, v := range s {
			if j%2 == 0 {
				v += dx
			}
			out[i][j] = int16(math.Round(v))
		}
	}
	return out
}

// proportional shifts a glyph so its ink starts at sideBearing and gives it a matching advance.
func proportional(strokes [][]float64) ink.Glyph {
	minX, maxX := xRange(strokes)
	return ink.Glyph{
		Advance: int16(math.Round(maxX - minX + 2*sideBearing)),
		Strokes: shifted(strokes, sideBearing-minX),
	}
}

// fullWidth keeps CJK glyphs near-square but tightens narrow ones (kana, punctuation) around
// their ink so lines read like natural Japanese handwriting.
func fullWidth(strokes [][]float64, r rune) ink.Glyph {
	isPunct := (r >= 0x3000 && r <= 0x303f) || (r >= 0xff00 && r <= 0xff0f)
	minX, maxX := xRange(strokes)
	minW := 0.8 * em
	if isPunct {
		minW = 0.55 * em
	}
	w := math.Round(math.Max(minW, math.Min(em, maxX-minX+180)))
	dx := (w-(maxX-minX))/2 - minX
	if isPunct {
		dx = 40 - minX
	}
	return ink.Glyph{Advance: int16(w), Strokes: shifted(strokes, dx)}
}

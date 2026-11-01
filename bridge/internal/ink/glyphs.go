// Package ink turns answer text into pen strokes that look handwritten.
package ink

import (
	"bufio"
	"compress/gzip"
	"encoding/binary"
	"errors"
	"fmt"
	"io"
	"os"
	"sort"
)

// Glyph is a centre-line character: polylines in stroke order, coordinates in em units
// (y grows downwards), each stored as x0,y0,x1,y1,….
type Glyph struct {
	Advance int16
	Strokes [][]int16
}

// Font is a set of centre-line glyphs sharing one em box.
type Font struct {
	Em      int16
	Sources []string
	Glyphs  map[rune]Glyph
}

const magic = "CLGF\x04" // bump whenever glyph generation changes

// ErrFormat means the file is not a glyph file of the current format (it should be rebuilt).
var ErrFormat = errors.New("not a glyph file of the current format")

// Write stores the font as a gzipped binary file. Points after the first in each stroke are
// stored as deltas, which are small and compress well.
func (f *Font) Write(w io.Writer) error {
	gz := gzip.NewWriter(w)
	bw := bufio.NewWriter(gz)
	put := func(v any) {
		_ = binary.Write(bw, binary.LittleEndian, v)
	}
	bw.WriteString(magic)
	put(f.Em)
	put(uint16(len(f.Sources)))
	for _, s := range f.Sources {
		put(uint16(len(s)))
		bw.WriteString(s)
	}
	runes := make([]rune, 0, len(f.Glyphs))
	for r := range f.Glyphs {
		runes = append(runes, r)
	}
	sort.Slice(runes, func(i, j int) bool { return runes[i] < runes[j] })
	put(uint32(len(runes)))
	for _, r := range runes {
		g := f.Glyphs[r]
		put(uint32(r))
		put(g.Advance)
		put(uint16(len(g.Strokes)))
		for _, s := range g.Strokes {
			put(uint16(len(s) / 2))
			put(delta(s))
		}
	}
	if err := bw.Flush(); err != nil {
		return err
	}
	return gz.Close()
}

func delta(s []int16) []int16 {
	out := make([]int16, len(s))
	for i := range s {
		out[i] = s[i]
		if i >= 2 {
			out[i] -= s[i-2]
		}
	}
	return out
}

func undelta(s []int16) {
	for i := 2; i < len(s); i++ {
		s[i] += s[i-2]
	}
}

// ReadFont loads a font written by Write.
func ReadFont(r io.Reader) (*Font, error) {
	gz, err := gzip.NewReader(r)
	if err != nil {
		return nil, ErrFormat
	}
	br := bufio.NewReader(gz)
	head := make([]byte, len(magic))
	if _, err := io.ReadFull(br, head); err != nil || string(head) != magic {
		return nil, ErrFormat
	}
	get := func(v any) {
		if err == nil {
			err = binary.Read(br, binary.LittleEndian, v)
		}
	}
	f := &Font{Glyphs: map[rune]Glyph{}}
	get(&f.Em)
	var nSources uint16
	get(&nSources)
	for i := 0; i < int(nSources) && err == nil; i++ {
		var n uint16
		get(&n)
		b := make([]byte, n)
		if err == nil {
			_, err = io.ReadFull(br, b)
		}
		f.Sources = append(f.Sources, string(b))
	}
	var count uint32
	get(&count)
	for i := 0; i < int(count) && err == nil; i++ {
		var cp uint32
		var g Glyph
		var nStrokes uint16
		get(&cp)
		get(&g.Advance)
		get(&nStrokes)
		g.Strokes = make([][]int16, nStrokes)
		for s := range g.Strokes {
			var nPts uint16
			get(&nPts)
			g.Strokes[s] = make([]int16, int(nPts)*2)
			get(g.Strokes[s])
			undelta(g.Strokes[s])
		}
		f.Glyphs[rune(cp)] = g
	}
	if err != nil {
		return nil, fmt.Errorf("corrupt glyph file: %w", err)
	}
	return f, nil
}

// LoadFont reads a glyph file from disk.
func LoadFont(path string) (*Font, error) {
	file, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer file.Close()
	return ReadFont(file)
}

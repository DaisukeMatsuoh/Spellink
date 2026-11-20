package main

import (
	"bufio"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"sort"
	"time"

	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/ink"
)

// The same three commands as experiments/bridge-rs, with the same input and output, so the Go
// and Rust implementations of the handwriting core can be compared exactly and measured alike.
// For measurement only; not part of what a user runs.

func composeFlags(name string, args []string) (*flag.FlagSet, *string, *string, *uint, *float64, *float64, *float64) {
	fs := flag.NewFlagSet(name, flag.ContinueOnError)
	glyphs := fs.String("glyphs", "", "glyph file")
	layout := fs.String("layout", "{}", "layout as JSON")
	seed := fs.Uint("seed", 0, "seed")
	slope := fs.Float64("slope", 0, "slope")
	full := fs.Float64("full", 0, "full-width spacing factor")
	half := fs.Float64("half", 0, "half-width spacing factor")
	return fs, glyphs, layout, seed, slope, full, half
}

func loadForCompose(glyphs, layoutJSON string, slope, full, half float64) (*ink.Font, ink.Layout, error) {
	font, err := ink.LoadFont(glyphs)
	if err != nil {
		return nil, ink.Layout{}, fmt.Errorf("%s: %w", glyphs, err)
	}
	var l ink.Layout
	if err := json.Unmarshal([]byte(layoutJSON), &l); err != nil {
		return nil, l, fmt.Errorf("--layout: %w", err)
	}
	l.Slope, l.SpacingFull, l.SpacingHalf = slope, full, half
	return font, l, nil
}

func composeCmd(args []string) error {
	fs, glyphs, layoutJSON, seed, slope, full, half := composeFlags("compose", args)
	if err := fs.Parse(args); err != nil {
		return err
	}
	font, l, err := loadForCompose(*glyphs, *layoutJSON, *slope, *full, *half)
	if err != nil {
		return err
	}
	c := ink.NewComposer(font, l, uint32(*seed))
	w := bufio.NewWriter(os.Stdout)
	defer w.Flush()
	enc := json.NewEncoder(w)
	enc.SetEscapeHTML(false)
	sc := bufio.NewScanner(os.Stdin)
	sc.Buffer(make([]byte, 64<<10), 4<<20)
	for sc.Scan() {
		if len(sc.Bytes()) == 0 {
			continue
		}
		var piece string
		if err := json.Unmarshal(sc.Bytes(), &piece); err != nil {
			return fmt.Errorf("input: %w", err)
		}
		for _, line := range c.Push(piece) {
			_ = enc.Encode(line)
		}
	}
	for _, line := range c.Finish() {
		_ = enc.Encode(line)
	}
	missing := []string{}
	for r := range c.Missing {
		missing = append(missing, string(r))
	}
	sort.Strings(missing)
	return enc.Encode(map[string]any{"summary": true, "missing": missing, "truncated": c.Truncated, "pages": c.Pages, "capacity": ink.Capacity(l)})
}

func benchCmd(args []string) error {
	fs, glyphs, layoutJSON, seed, slope, full, half := composeFlags("bench", args)
	textFile := fs.String("text", "", "answer text")
	n := fs.Int("iterations", 100, "iterations")
	if err := fs.Parse(args); err != nil {
		return err
	}
	font, l, err := loadForCompose(*glyphs, *layoutJSON, *slope, *full, *half)
	if err != nil {
		return err
	}
	b, err := os.ReadFile(*textFile)
	if err != nil {
		return err
	}
	runes := []rune(string(b))
	started := time.Now()
	strokes := 0
	for i := 0; i < *n; i++ {
		c := ink.NewComposer(font, l, uint32(*seed)+uint32(i))
		for j := 0; j < len(runes); j += 8 {
			for _, line := range c.Push(string(runes[j:min(j+8, len(runes))])) {
				strokes += len(line.Strokes)
			}
		}
		for _, line := range c.Finish() {
			strokes += len(line.Strokes)
		}
	}
	ms := float64(time.Since(started).Microseconds()) / 1000
	fmt.Printf("{\"impl\":\"go\",\"iterations\":%d,\"ms\":%.1f,\"msPerAnswer\":%.3f,\"strokes\":%d}\n", *n, ms, ms/float64(*n), strokes)
	return nil
}

func idleCmd(args []string) error {
	fs := flag.NewFlagSet("idle", flag.ContinueOnError)
	glyphs := fs.String("glyphs", "", "glyph file")
	secs := fs.Int("seconds", 5, "seconds")
	if err := fs.Parse(args); err != nil {
		return err
	}
	font, err := ink.LoadFont(*glyphs)
	if err != nil {
		return err
	}
	fmt.Fprintf(os.Stderr, "loaded %d glyphs; waiting %d s\n", len(font.Glyphs), *secs)
	time.Sleep(time.Duration(*secs) * time.Second)
	return nil
}

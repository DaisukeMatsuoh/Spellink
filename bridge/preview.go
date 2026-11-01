package main

import (
	"errors"
	"fmt"
	"image"
	"image/color"
	"image/png"
	"math"
	"os"
	"strconv"

	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/ink"
)

// preview renders text as handwriting onto Nomad-sized PNG pages, to check output without a device.
func preview(args []string) error {
	if len(args) < 2 {
		return errors.New("preview TEXT OUT.png [charSize] [maxPages]")
	}
	size, pages := 44.0, 1
	if len(args) > 2 {
		size, _ = strconv.ParseFloat(args[2], 64)
	}
	if len(args) > 3 {
		pages, _ = strconv.Atoi(args[3])
	}
	font, err := loadFont()
	if err != nil {
		return err
	}
	const w, h = 1404, 1872 // Nomad
	c := ink.NewComposer(font, ink.Layout{
		X: 80, Y: 80, Width: w - 160, PageHeight: h, BottomMargin: 80, CharSize: size,
		MaxPages: pages, ContinueTop: 80, Slope: -math.Tan(1.0 * math.Pi / 180), // 1° rising
	}, 1)
	lines := append(c.Push(args[0]), c.Finish()...)

	// Pages are drawn side by side.
	img := image.NewGray(image.Rect(0, 0, w*c.Pages, h))
	for i := range img.Pix {
		img.Pix[i] = 255
	}
	points := 0
	for _, l := range lines {
		for _, s := range l.Strokes {
			points += len(s.P) / 2
			drawStroke(img, s, float64(l.Page*w))
		}
	}
	f, err := os.Create(args[1])
	if err != nil {
		return err
	}
	defer f.Close()
	fmt.Printf("%d lines on %d page(s), %d points, truncated=%v, missing=%d\n",
		len(lines), c.Pages, points, c.Truncated, len(c.Missing))
	return png.Encode(f, img)
}

func drawStroke(img *image.Gray, s ink.Stroke, dx float64) {
	dot := func(x, y, r float64) {
		for yy := int(y - r); yy <= int(y+r); yy++ {
			for xx := int(x - r); xx <= int(x+r); xx++ {
				if (float64(xx)-x)*(float64(xx)-x)+(float64(yy)-y)*(float64(yy)-y) <= r*r {
					img.SetGray(xx, yy, color.Gray{})
				}
			}
		}
	}
	for i := 0; i+3 < len(s.P); i += 2 {
		x0, y0 := float64(s.P[i])+dx, float64(s.P[i+1])
		x1, y1 := float64(s.P[i+2])+dx, float64(s.P[i+3])
		r := 0.6 + float64(s.W[i/2])/4096*1.6
		n := max(1, int(math.Ceil(math.Hypot(x1-x0, y1-y0))))
		for k := 0; k <= n; k++ {
			t := float64(k) / float64(n)
			dot(x0+(x1-x0)*t, y0+(y1-y0)*t, r)
		}
	}
}

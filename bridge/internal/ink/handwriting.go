package ink

import "math"

// Stroke is one pen stroke in page pixels: P = x0,y0,x1,y1,…; W = pressure per point (0..4096).
type Stroke struct {
	P []int `json:"p"`
	W []int `json:"w"`
}

const maxPressure = 4096

// Variation applied to every glyph, as fractions of the character size.
const (
	jitterScale        = 0.03
	jitterRotate       = 0.025 // radians
	jitterOffset       = 0.035
	jitterStrokeOffset = 0.012
)

// RenderGlyph turns a glyph into pen strokes with its em box's top-left at (x, top), size px tall.
// Points are resampled at a roughly constant spacing, the way a digitizer samples a real pen.
func RenderGlyph(g Glyph, em int16, x, top, size float64, rng *Random) []Stroke {
	scale := size / float64(em) * (1 + rng.Gaussian()*jitterScale)
	angle := rng.Gaussian() * jitterRotate
	cos, sin := math.Cos(angle), math.Sin(angle)
	cx, cy := float64(g.Advance)/2*scale, float64(em)/2*scale
	ox := x + rng.Gaussian()*jitterOffset*size
	oy := top + rng.Gaussian()*jitterOffset*size
	spacing := math.Max(1.5, size/30)

	out := make([]Stroke, 0, len(g.Strokes))
	for _, poly := range g.Strokes {
		sx := rng.Gaussian() * jitterStrokeOffset * size
		sy := rng.Gaussian() * jitterStrokeOffset * size
		pts := make([]float64, len(poly))
		for i := 0; i < len(poly); i += 2 {
			px, py := float64(poly[i])*scale-cx, float64(poly[i+1])*scale-cy
			pts[i] = ox + cx + px*cos - py*sin + sx
			pts[i+1] = oy + cy + px*sin + py*cos + sy
		}
		res := Resample(pts, spacing)
		p := make([]int, len(res))
		for i, v := range res {
			p[i] = int(math.Round(v))
		}
		out = append(out, Stroke{P: p, W: pressureProfile(len(p)/2, rng)})
	}
	return out
}

// Resample returns points spaced step apart along the polyline, keeping both ends.
func Resample(pts []float64, step float64) []float64 {
	if len(pts) <= 2 {
		return append([]float64(nil), pts...)
	}
	out := []float64{pts[0], pts[1]}
	carry := 0.0
	for i := 2; i < len(pts); i += 2 {
		x0, y0, x1, y1 := pts[i-2], pts[i-1], pts[i], pts[i+1]
		seg := math.Hypot(x1-x0, y1-y0)
		d := step - carry
		for d <= seg {
			t := d / seg
			out = append(out, x0+(x1-x0)*t, y0+(y1-y0)*t)
			d += step
		}
		carry = seg - (d - step)
	}
	lx, ly := pts[len(pts)-2], pts[len(pts)-1]
	if out[len(out)-2] != lx || out[len(out)-1] != ly {
		out = append(out, lx, ly)
	}
	return out
}

// pressureProfile rises at the start and eases off at the end, like a pen landing and lifting.
func pressureProfile(n int, rng *Random) []int {
	base := 0.62 + rng.Gaussian()*0.05
	ramp := max(1, int(math.Round(float64(n)*0.12)))
	w := make([]int, n)
	for i := range w {
		in := math.Min(1, float64(i+1)/float64(ramp))
		out := math.Min(1, float64(n-i)/float64(ramp))
		f := 0.35 + 0.65*math.Min(in, out)
		w[i] = int(math.Round(math.Min(1, base*f) * maxPressure))
	}
	return w
}

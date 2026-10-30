package ink

import "math"

// Random is a small seeded PRNG (mulberry32), so one answer always yields the same handwriting.
type Random struct{ state uint32 }

func NewRandom(seed uint32) *Random { return &Random{state: seed} }

func (r *Random) Float() float64 {
	r.state += 0x6d2b79f5
	t := r.state
	t = (t ^ (t >> 15)) * (t | 1)
	t ^= t + (t^(t>>7))*(t|61)
	return float64(t^(t>>14)) / 4294967296
}

// Gaussian returns a standard normal sample clamped to ±3σ so outliers never wreck a glyph.
func (r *Random) Gaussian() float64 {
	u := 1 - r.Float()
	v := r.Float()
	g := math.Sqrt(-2*math.Log(u)) * math.Cos(2*math.Pi*v)
	return math.Max(-3, math.Min(3, g))
}

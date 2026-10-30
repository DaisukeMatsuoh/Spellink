package glyphbuild

import (
	"fmt"
	"math"
	"regexp"
	"strconv"
)

// Max distance (em units) between a curve and its polyline approximation: 0.4% of the
// character size, well below a pixel at any size the plugin writes.
const flattenTolerance = 4.0

var pathToken = regexp.MustCompile(`[MmLlHhVvCcSsQqTtZz]|[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?`)

type point struct{ x, y float64 }

// PathToPolylines flattens an SVG path into polylines (one per subpath), scaling every
// coordinate by scale. Supports the commands KanjiVG and Hershey use: M L H V C S Q T Z.
func PathToPolylines(d string, scale float64) ([][]float64, error) {
	tokens := pathToken.FindAllString(d, -1)
	var lines [][]float64
	var cur []float64
	var pos, start, lastCtrl point
	var cmd byte
	prevCmd := byte(0)
	i := 0
	num := func() (float64, error) {
		if i >= len(tokens) {
			return 0, fmt.Errorf("path %q: missing number", d)
		}
		v, err := strconv.ParseFloat(tokens[i], 64)
		if err != nil {
			return 0, fmt.Errorf("path %q: %w", d, err)
		}
		i++
		return v * scale, nil
	}
	pt := func(rel bool) (point, error) {
		x, err := num()
		if err != nil {
			return point{}, err
		}
		y, err := num()
		if err != nil {
			return point{}, err
		}
		if rel {
			x, y = x+pos.x, y+pos.y
		}
		return point{x, y}, nil
	}
	flushLine := func() {
		if len(cur) >= 4 {
			lines = append(lines, cur)
		}
		cur = nil
	}
	for i < len(tokens) {
		t := tokens[i]
		if len(t) == 1 && (t[0] >= 'A' && t[0] <= 'Z' || t[0] >= 'a' && t[0] <= 'z') {
			cmd = t[0]
			i++
		} else if cmd|0x20 == 'z' || cmd == 0 {
			return nil, fmt.Errorf("path %q: number without a command", d)
		} else if cmd == 'M' {
			cmd = 'L' // implicit lineto after moveto
		} else if cmd == 'm' {
			cmd = 'l'
		}
		rel := cmd >= 'a'
		var err error
		switch cmd | 0x20 { // lower-case
		case 'm':
			flushLine()
			if pos, err = pt(rel); err != nil {
				return nil, err
			}
			start = pos
			cur = []float64{pos.x, pos.y}
		case 'l':
			if pos, err = pt(rel); err != nil {
				return nil, err
			}
			cur = append(cur, pos.x, pos.y)
		case 'h':
			x, err := num()
			if err != nil {
				return nil, err
			}
			if rel {
				x += pos.x
			}
			pos.x = x
			cur = append(cur, pos.x, pos.y)
		case 'v':
			y, err := num()
			if err != nil {
				return nil, err
			}
			if rel {
				y += pos.y
			}
			pos.y = y
			cur = append(cur, pos.x, pos.y)
		case 'c', 's':
			var c1 point
			if cmd|0x20 == 'c' {
				if c1, err = pt(rel); err != nil {
					return nil, err
				}
			} else if prevCmd|0x20 == 'c' || prevCmd|0x20 == 's' {
				c1 = point{2*pos.x - lastCtrl.x, 2*pos.y - lastCtrl.y}
			} else {
				c1 = pos
			}
			c2, err := pt(rel)
			if err != nil {
				return nil, err
			}
			end, err := pt(rel)
			if err != nil {
				return nil, err
			}
			cur = flattenCubic(pos, c1, c2, end, cur)
			lastCtrl, pos = c2, end
		case 'q', 't':
			var q point
			if cmd|0x20 == 'q' {
				if q, err = pt(rel); err != nil {
					return nil, err
				}
			} else if prevCmd|0x20 == 'q' || prevCmd|0x20 == 't' {
				q = point{2*pos.x - lastCtrl.x, 2*pos.y - lastCtrl.y}
			} else {
				q = pos
			}
			end, err := pt(rel)
			if err != nil {
				return nil, err
			}
			c1 := point{pos.x + 2.0/3*(q.x-pos.x), pos.y + 2.0/3*(q.y-pos.y)}
			c2 := point{end.x + 2.0/3*(q.x-end.x), end.y + 2.0/3*(q.y-end.y)}
			cur = flattenCubic(pos, c1, c2, end, cur)
			lastCtrl, pos = q, end
		case 'z':
			cur = append(cur, start.x, start.y)
			pos = start
		default:
			return nil, fmt.Errorf("path %q: unsupported command %q", d, cmd)
		}
		prevCmd = cmd
	}
	flushLine()
	return lines, nil
}

func flattenCubic(p0, p1, p2, p3 point, out []float64) []float64 {
	l := math.Hypot(p1.x-p0.x, p1.y-p0.y) + math.Hypot(p2.x-p1.x, p2.y-p1.y) + math.Hypot(p3.x-p2.x, p3.y-p2.y)
	n := max(1, int(math.Ceil(math.Sqrt(l/flattenTolerance))))
	for k := 1; k <= n; k++ {
		t := float64(k) / float64(n)
		u := 1 - t
		a, b, c, d := u*u*u, 3*u*u*t, 3*u*t*t, t*t*t
		out = append(out, a*p0.x+b*p1.x+c*p2.x+d*p3.x, a*p0.y+b*p1.y+c*p2.y+d*p3.y)
	}
	return out
}

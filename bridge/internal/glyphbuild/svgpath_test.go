package glyphbuild

import (
	"math"
	"testing"
)

func near(t *testing.T, got, want []float64) {
	t.Helper()
	if len(got) != len(want) {
		t.Fatalf("got %v, want %v", got, want)
	}
	for i := range got {
		if math.Abs(got[i]-want[i]) > 1e-9 {
			t.Fatalf("got %v, want %v", got, want)
		}
	}
}

func TestLinesAndImplicitLineto(t *testing.T) {
	lines, err := PathToPolylines("M4,1 L4,22 M4,1 L13,1 16,2", 1)
	if err != nil {
		t.Fatal(err)
	}
	if len(lines) != 2 {
		t.Fatalf("want 2 subpaths, got %d", len(lines))
	}
	near(t, lines[1], []float64{4, 1, 13, 1, 16, 2})
}

func TestRelativeCubicWithPackedNumbers(t *testing.T) {
	// KanjiVG style: negative numbers act as separators.
	lines, err := PathToPolylines("M54.5,15.79c0,6.07-0.29,55.49-0.29,60.55", 1)
	if err != nil {
		t.Fatal(err)
	}
	l := lines[0]
	near(t, l[len(l)-2:], []float64{54.21, 76.34})
	near(t, l[:2], []float64{54.5, 15.79})
}

func TestSmoothCubicReflectsControlPoint(t *testing.T) {
	// A symmetric S-curve ends where expected.
	lines, err := PathToPolylines("M0,0 C0,10 10,10 10,0 S20,-10 20,0", 2)
	if err != nil {
		t.Fatal(err)
	}
	l := lines[0]
	near(t, l[len(l)-2:], []float64{40, 0})
}

func TestRejectsGarbage(t *testing.T) {
	for _, d := range []string{"1,2", "M0,0 Z 5,5", "M0,0 A1,1 0 0 0 2,2"} {
		if _, err := PathToPolylines(d, 1); err == nil {
			t.Errorf("%q: expected an error", d)
		}
	}
}

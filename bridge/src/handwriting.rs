//! `bridge/internal/ink/handwriting.go`: glyph → pen strokes, with the same variation.

use crate::glyphs::Glyph;
use crate::random::Random;
use serde::Serialize;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Stroke {
    pub p: Vec<i64>,
    pub w: Vec<i64>,
}

const MAX_PRESSURE: f64 = 4096.0;
const JITTER_SCALE: f64 = 0.03;
const JITTER_ROTATE: f64 = 0.025;
const JITTER_OFFSET: f64 = 0.035;
const JITTER_STROKE_OFFSET: f64 = 0.012;

/// Go's math.Round: half away from zero (Rust's f64::round is the same).
fn round(v: f64) -> i64 {
    v.round() as i64
}

pub fn render_glyph(g: &Glyph, em: i16, x: f64, top: f64, size: f64, rng: &mut Random) -> Vec<Stroke> {
    let scale = size / em as f64 * (1.0 + rng.gaussian() * JITTER_SCALE);
    let angle = rng.gaussian() * JITTER_ROTATE;
    let (cos, sin) = (angle.cos(), angle.sin());
    let (cx, cy) = (g.advance as f64 / 2.0 * scale, em as f64 / 2.0 * scale);
    let ox = x + rng.gaussian() * JITTER_OFFSET * size;
    let oy = top + rng.gaussian() * JITTER_OFFSET * size;
    let spacing = f64::max(1.5, size / 30.0);

    let mut out = Vec::with_capacity(g.strokes.len());
    for poly in &g.strokes {
        let sx = rng.gaussian() * JITTER_STROKE_OFFSET * size;
        let sy = rng.gaussian() * JITTER_STROKE_OFFSET * size;
        let mut pts = vec![0.0; poly.len()];
        let mut i = 0;
        while i + 1 < poly.len() {
            let (px, py) = (poly[i] as f64 * scale - cx, poly[i + 1] as f64 * scale - cy);
            pts[i] = ox + cx + px * cos - py * sin + sx;
            pts[i + 1] = oy + cy + px * sin + py * cos + sy;
            i += 2;
        }
        let p: Vec<i64> = resample(&pts, spacing).into_iter().map(round).collect();
        let w = pressure_profile(p.len() / 2, rng);
        out.push(Stroke { p, w });
    }
    out
}

/// Points spaced `step` apart along the polyline, keeping both ends.
pub fn resample(pts: &[f64], step: f64) -> Vec<f64> {
    if pts.len() <= 2 {
        return pts.to_vec();
    }
    let mut out = vec![pts[0], pts[1]];
    let mut carry = 0.0;
    let mut i = 2;
    while i + 1 < pts.len() {
        let (x0, y0, x1, y1) = (pts[i - 2], pts[i - 1], pts[i], pts[i + 1]);
        let seg = (x1 - x0).hypot(y1 - y0);
        let mut d = step - carry;
        while d <= seg {
            let t = d / seg;
            out.push(x0 + (x1 - x0) * t);
            out.push(y0 + (y1 - y0) * t);
            d += step;
        }
        carry = seg - (d - step);
        i += 2;
    }
    let (lx, ly) = (pts[pts.len() - 2], pts[pts.len() - 1]);
    if out[out.len() - 2] != lx || out[out.len() - 1] != ly {
        out.push(lx);
        out.push(ly);
    }
    out
}

fn pressure_profile(n: usize, rng: &mut Random) -> Vec<i64> {
    let base = 0.62 + rng.gaussian() * 0.05;
    let ramp = std::cmp::max(1, (n as f64 * 0.12).round() as i64) as f64;
    (0..n)
        .map(|i| {
            let inn = f64::min(1.0, (i + 1) as f64 / ramp);
            let out = f64::min(1.0, (n - i) as f64 / ramp);
            let f = 0.35 + 0.65 * f64::min(inn, out);
            round(f64::min(1.0, base * f) * MAX_PRESSURE)
        })
        .collect()
}

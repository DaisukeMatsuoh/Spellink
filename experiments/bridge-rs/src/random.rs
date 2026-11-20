//! mulberry32, exactly as `bridge/internal/ink/random.go`, so the same answer gives the same handwriting.

pub struct Random {
    state: u32,
}

impl Random {
    pub fn new(seed: u32) -> Self {
        Random { state: seed }
    }

    pub fn float(&mut self) -> f64 {
        self.state = self.state.wrapping_add(0x6d2b79f5);
        let mut t = self.state;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        (t ^ (t >> 14)) as f64 / 4294967296.0
    }

    /// A standard normal sample clamped to ±3σ.
    pub fn gaussian(&mut self) -> f64 {
        let u = 1.0 - self.float();
        let v = self.float();
        let g = (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos();
        g.clamp(-3.0, 3.0)
    }
}

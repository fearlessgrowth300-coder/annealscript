//! Minimal interval abstraction backing the safety solver in `solver.rs`.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Interval {
    pub lo: f64,
    pub hi: f64,
}

impl Interval {
    pub const UNKNOWN: Interval = Interval { lo: f64::NEG_INFINITY, hi: f64::INFINITY };

    pub fn point(v: f64) -> Self {
        Interval { lo: v, hi: v }
    }

    pub fn is_unknown(&self) -> bool {
        self.lo == f64::NEG_INFINITY && self.hi == f64::INFINITY
    }

    pub fn violates_le(&self, limit: f64) -> bool {
        self.hi > limit
    }

    pub fn violates_ge(&self, limit: f64) -> bool {
        self.lo < limit
    }
}

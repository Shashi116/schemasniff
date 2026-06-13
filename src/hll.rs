//! Minimal HyperLogLog++ cardinality estimator.
//!
//! Precision p=14 → 2^14 = 16384 registers → ±0.8% error at 95% confidence.
//! Uses a 64-bit hash (FNV-1a) — fast, no external deps, not cryptographic.
//! Values are never stored — only the hash is observed, making this safe
//! for processing sensitive data.

/// HyperLogLog++ estimator with precision p=14.
/// One instance per column; call `insert` for each non-null cell value,
/// then `count` at the end to get the cardinality estimate.
pub struct Hll {
    registers: Vec<u8>,
}

impl Hll {
    /// Number of registers: 2^14 = 16384
    const P: u32 = 14;
    const M: usize = 1 << Self::P; // 16384
    const ALPHA: f64 = 0.7213 / (1.0 + 1.079 / Self::M as f64);

    pub fn new() -> Self {
        Self {
            registers: vec![0u8; Self::M],
        }
    }

    /// Observe a string value. Hashes it and updates the sketch.
    /// The raw string is not retained after this call.
    pub fn insert(&mut self, value: &str) {
        let hash = fnv1a_64(value.as_bytes());
        // Top P bits → register index
        let index = (hash >> (64 - Self::P)) as usize;
        // Remaining bits → position of leftmost 1-bit (rho)
        let w   = hash << Self::P;
        let rho = w.leading_zeros() + 1;
        // SAFETY: index is derived from top P=14 bits of a 64-bit hash,
        // so index < 2^14 = M. The vec is always allocated with M elements.
        #[allow(clippy::indexing_slicing)]
        let reg = &mut self.registers[index];
        if rho as u8 > *reg {
            *reg = rho as u8;
        }
    }

    /// Estimate the number of distinct values observed.
    pub fn count(&self) -> f64 {
        let m = Self::M as f64;
        let raw: f64 = Self::ALPHA * m * m
            / self.registers.iter().map(|&r| 2f64.powi(-(r as i32))).sum::<f64>();

        // Small range correction
        if raw <= 2.5 * m {
            let zeros = self.registers.iter().filter(|&&r| r == 0).count() as f64;
            if zeros > 0.0 {
                return m * (m / zeros).ln();
            }
        }

        // Large range correction (2^32 limit)
        if raw <= (1u64 << 32) as f64 / 30.0 {
            raw
        } else {
            -((1u64 << 32) as f64) * (1.0 - raw / (1u64 << 32) as f64).ln()
        }
    }
}

impl Default for Hll {
    fn default() -> Self {
        Self::new()
    }
}

/// FNV-1a 64-bit hash with MurmurHash3 finalizer for better avalanche.
/// Not cryptographic — used only for HLL bucket distribution.
fn fnv1a_64(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME:  u64 = 0x00000100000001b3;
    let mut h = OFFSET;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(PRIME);
    }
    // MurmurHash3 64-bit finalizer — improves avalanche for short keys
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51afd7ed558ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ceb9fe1a85ec53);
    h ^= h >> 33;
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_sketch_returns_zero() {
        let hll = Hll::new();
        assert_eq!(hll.count().round() as u64, 0);
    }

    #[test]
    fn single_value_returns_one() {
        let mut hll = Hll::new();
        hll.insert("hello");
        assert!((hll.count() - 1.0).abs() < 1.0);
    }

    #[test]
    fn duplicate_values_do_not_inflate_count() {
        let mut hll = Hll::new();
        for _ in 0..1000 {
            hll.insert("same_value");
        }
        // Should be ~1, not 1000
        assert!(hll.count() < 10.0);
    }

    #[test]
    fn cardinality_within_2_percent_at_10k() {
        let mut hll = Hll::new();
        for i in 0..10_000 {
            hll.insert(&i.to_string());
        }
        let estimate = hll.count();
        let error = (estimate - 10_000.0).abs() / 10_000.0;
        assert!(error < 0.02, "error {error:.4} exceeded 2% at 10k distinct values");
    }

    #[test]
    fn cardinality_within_2_percent_at_100k() {
        let mut hll = Hll::new();
        for i in 0..100_000 {
            hll.insert(&i.to_string());
        }
        let estimate = hll.count();
        let error = (estimate - 100_000.0).abs() / 100_000.0;
        assert!(error < 0.02, "error {error:.4} exceeded 2% at 100k distinct values");
    }
}

//! Randomness with no dependencies.
//!
//! Two kinds of draw. `SplitMix64` is a sequential stream, used to build a
//! run's fault history once. `keyed_unit` is stateless: the same key always
//! gives the same number. Upsets are drawn that way, keyed by unit and day, so
//! every policy sees exactly the same upsets no matter which days it lives to
//! read. That is what makes the comparison paired rather than merely averaged.

pub struct SplitMix64(u64);

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        mix(self.0)
    }

    /// Uniform in [0, 1).
    pub fn next_f64(&mut self) -> f64 {
        unit(self.next_u64())
    }

    /// Uniform in [0, n). `n` is tiny here, so modulo bias is irrelevant.
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

/// The SplitMix64 finaliser. A good 64-bit mixer on its own.
pub fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Uniform in [0, 1) from the top 53 bits.
pub fn unit(x: u64) -> f64 {
    (x >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
}

/// A uniform draw that depends only on its key.
pub fn keyed_unit(seed: u64, a: u64, b: u64) -> f64 {
    unit(mix(seed ^ mix(a ^ mix(b.wrapping_add(0x9E37_79B9_7F4A_7C15)))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_is_in_range_and_centred() {
        let mut r = SplitMix64::new(1);
        let n = 200_000;
        let mut sum = 0.0;
        for _ in 0..n {
            let x = r.next_f64();
            assert!((0.0..1.0).contains(&x));
            sum += x;
        }
        assert!((sum / n as f64 - 0.5).abs() < 0.005);
    }

    #[test]
    fn keyed_draws_are_stable_and_spread() {
        assert_eq!(keyed_unit(7, 3, 11), keyed_unit(7, 3, 11));
        let n = 100_000u64;
        let hits = (0..n).filter(|&d| keyed_unit(7, 0, d) < 0.01).count();
        assert!((hits as f64 / n as f64 - 0.01).abs() < 0.002);
    }
}

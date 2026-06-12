//! Numerical primitives for pricing math.

use std::f64::consts::{SQRT_2, TAU};

/// Standard normal probability density: φ(x) = e^(−x²/2) / √(2π).
pub fn norm_pdf(x: f64) -> f64 {
    (-0.5 * x * x).exp() / TAU.sqrt()
}

/// Standard normal cumulative distribution: Φ(x) = (1 + erf(x/√2)) / 2.
pub fn norm_cdf(x: f64) -> f64 {
    0.5 * (1.0 + libm::erf(x / SQRT_2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_at_zero() {
        // 1/sqrt(2*pi)
        assert!((norm_pdf(0.0) - 0.3989422804014327).abs() < 1e-15);
    }

    #[test]
    fn pdf_symmetric() {
        assert!((norm_pdf(1.5) - norm_pdf(-1.5)).abs() < 1e-15);
    }

    #[test]
    fn cdf_at_zero() {
        assert!((norm_cdf(0.0) - 0.5).abs() < 1e-15);
    }

    #[test]
    fn cdf_complementary() {
        for x in [0.1, 0.5, 1.0, 2.0, 5.0] {
            assert!((norm_cdf(-x) + norm_cdf(x) - 1.0).abs() < 1e-15);
        }
    }

    #[test]
    fn cdf_known_values() {
        // Φ(1.96) ≈ 0.975 (the classic 95% two-sided quantile)
        assert!((norm_cdf(1.959963984540054) - 0.975).abs() < 1e-12);
        // Φ(1) ≈ 0.8413447460685429
        assert!((norm_cdf(1.0) - 0.8413447460685429).abs() < 1e-12);
    }

    #[test]
    fn cdf_tails() {
        assert!(norm_cdf(-10.0) < 1e-20);
        // (1.0 - 1e-20 is not representable in f64 — it rounds to 1.0)
        assert!(norm_cdf(10.0) >= 1.0 - 1e-15);
    }
}

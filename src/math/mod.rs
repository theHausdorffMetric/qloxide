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

/// Find a root of `f` in `[a, b]` with Brent's method (Numerical Recipes
/// `zbrent`): bisection safety with inverse-quadratic/secant speed.
///
/// Requires `f(a)` and `f(b)` to bracket a root (opposite signs or an
/// exact zero at an endpoint). Returns `None` if the bracket is invalid,
/// `f` returns NaN, or `max_iter` is exhausted before the interval
/// shrinks below `tol`. Chosen over Newton-Raphson deliberately: NR on
/// implied-vol objectives diverges to negative vols near zero vega.
pub fn brent_root(
    f: impl Fn(f64) -> f64,
    a: f64,
    b: f64,
    tol: f64,
    max_iter: usize,
) -> Option<f64> {
    let (mut a, mut b) = (a, b);
    let mut fa = f(a);
    let mut fb = f(b);
    if fa.is_nan() || fb.is_nan() || fa * fb > 0.0 {
        return None;
    }
    let (mut c, mut fc) = (a, fa);
    let (mut d, mut e) = (b - a, b - a);
    for _ in 0..max_iter {
        if fb * fc > 0.0 {
            // Root no longer between b and c: reset c to the old a side.
            c = a;
            fc = fa;
            d = b - a;
            e = d;
        }
        if fc.abs() < fb.abs() {
            // Keep b as the best estimate.
            a = b;
            b = c;
            c = a;
            fa = fb;
            fb = fc;
            fc = fa;
        }
        let tol1 = 2.0 * f64::EPSILON * b.abs() + 0.5 * tol;
        let xm = 0.5 * (c - b);
        if xm.abs() <= tol1 || fb == 0.0 {
            return Some(b);
        }
        if e.abs() >= tol1 && fa.abs() > fb.abs() {
            // Attempt inverse quadratic interpolation (secant if a == c).
            let s = fb / fa;
            let (mut p, mut q);
            if a == c {
                p = 2.0 * xm * s;
                q = 1.0 - s;
            } else {
                let q0 = fa / fc;
                let r = fb / fc;
                p = s * (2.0 * xm * q0 * (q0 - r) - (b - a) * (r - 1.0));
                q = (q0 - 1.0) * (r - 1.0) * (s - 1.0);
            }
            if p > 0.0 {
                q = -q;
            }
            p = p.abs();
            let min1 = 3.0 * xm * q - (tol1 * q).abs();
            let min2 = (e * q).abs();
            if 2.0 * p < min1.min(min2) {
                // Interpolation acceptable.
                e = d;
                d = p / q;
            } else {
                // Fall back to bisection.
                d = xm;
                e = d;
            }
        } else {
            d = xm;
            e = d;
        }
        a = b;
        fa = fb;
        if d.abs() > tol1 {
            b += d;
        } else {
            b += tol1.copysign(xm);
        }
        fb = f(b);
        if fb.is_nan() {
            return None;
        }
    }
    None
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

    #[test]
    fn brent_finds_simple_roots() {
        // x³ + x² − 4 = 0 has a root at ≈ 1.3146
        let r = brent_root(|x| x.powi(3) + x.powi(2) - 4.0, 0.0, 10.0, 1e-14, 100).unwrap();
        assert!((r.powi(3) + r.powi(2) - 4.0).abs() < 1e-12, "got {r}");
        // cos(x) = x
        let r = brent_root(|x| x.cos() - x, 0.0, 1.0, 1e-14, 100).unwrap();
        assert!((r - 0.7390851332151607).abs() < 1e-12, "got {r}");
    }

    #[test]
    fn brent_accepts_endpoint_roots() {
        let r = brent_root(|x| x - 2.0, 2.0, 5.0, 1e-14, 100).unwrap();
        assert!((r - 2.0).abs() < 1e-12);
    }

    #[test]
    fn brent_rejects_bad_brackets() {
        // Same sign at both ends: no bracket.
        assert!(brent_root(|x| x * x + 1.0, -1.0, 1.0, 1e-14, 100).is_none());
        // NaN in the objective.
        assert!(brent_root(|_| f64::NAN, -1.0, 1.0, 1e-14, 100).is_none());
    }

    #[test]
    fn brent_respects_max_iter() {
        // One iteration cannot resolve this root to 1e-14.
        assert!(brent_root(|x| x.cos() - x, 0.0, 1.0, 1e-14, 1).is_none());
    }
}

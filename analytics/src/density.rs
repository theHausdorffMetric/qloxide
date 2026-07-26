//! Breeden-Litzenberger extraction from a fitted call spline.
//!
//! In forward space: q(K) = C̃''(K) and F(K) = 1 + C̃'(K) — the digital-
//! call identity, which validates the CDF without a second derivative.
//! For the natural cubic spline, C̃'' is the piecewise-linear function
//! through the knot values γ, so the density is exact, not re-differenced.
//!
//! The interior carries mass C̃'(uₙ) − C̃'(u₁) < 1; the remainder sits
//! beyond the strike range. This pre-graft object reports summary moments
//! with that mass as atoms at the boundary strikes — biased toward the
//! interior by construction. [`crate::tails::graft`] replaces the atoms
//! with fitted GPD tails.

use crate::fengler::FenglerFit;

#[derive(Clone, Debug)]
pub struct Density {
    /// Knots (strikes).
    pub u: Vec<f64>,
    /// Density at the knots (= γ); piecewise linear between them.
    pub q: Vec<f64>,
    /// CDF at the knots via the digital identity F = 1 + C̃'.
    pub cdf: Vec<f64>,
    pub mass_interior: f64,
    /// P(S < u₁) implied by the entry slope.
    pub mass_left: f64,
    /// P(S > uₙ) implied by the exit slope.
    pub mass_right: f64,
    /// Moments of interior + boundary atoms (pre-tail approximation).
    pub mean: f64,
    pub sd: f64,
    pub skew: f64,
    pub kurt: f64,
}

pub fn extract(fit: &FenglerFit) -> Density {
    let u = &fit.u;
    let g = &fit.g;
    let ga = &fit.gamma;
    let n = u.len();
    let h: Vec<f64> = u.windows(2).map(|w| w[1] - w[0]).collect();

    // Spline first derivative at each knot: right-sided form on interior
    // intervals, left-sided at the final knot (C¹ makes them consistent).
    let mut cp = vec![0.0; n];
    for i in 0..n - 1 {
        cp[i] = (g[i + 1] - g[i]) / h[i] - h[i] * (2.0 * ga[i] + ga[i + 1]) / 6.0;
    }
    cp[n - 1] = (g[n - 1] - g[n - 2]) / h[n - 2] + h[n - 2] * (ga[n - 2] + 2.0 * ga[n - 1]) / 6.0;

    let cdf: Vec<f64> = cp.iter().map(|d| (1.0 + d).clamp(0.0, 1.0)).collect();
    let mass_left = cdf[0];
    let mass_right = 1.0 - cdf[n - 1];
    let mass_interior = cdf[n - 1] - cdf[0];

    let mut raw = raw_moments_interior(u, ga, 0, n - 1);
    // Boundary atoms for the unmodelled tail mass.
    for (kk, r) in raw.iter_mut().enumerate() {
        *r += mass_left * u[0].powi(kk as i32) + mass_right * u[n - 1].powi(kk as i32);
    }

    let mean = raw[1];
    let var = (raw[2] - mean * mean).max(0.0);
    let sd = var.sqrt();
    let m3 = raw[3] - 3.0 * mean * raw[2] + 2.0 * mean.powi(3);
    let m4 = raw[4] - 4.0 * mean * raw[3] + 6.0 * mean * mean * raw[2] - 3.0 * mean.powi(4);
    let skew = if sd > 0.0 { m3 / sd.powi(3) } else { f64::NAN };
    let kurt = if sd > 0.0 { m4 / sd.powi(4) } else { f64::NAN };

    Density {
        u: u.clone(),
        q: ga.clone(),
        cdf,
        mass_interior,
        mass_left,
        mass_right,
        mean,
        sd,
        skew,
        kurt,
    }
}

impl Density {
    /// Quantile by inverting the piecewise-quadratic CDF (bisection on
    /// the bracketing interval). Returns None if p falls in a tail.
    pub fn quantile(&self, p: f64) -> Option<f64> {
        quantile_interior(&self.u, &self.q, &self.cdf, p)
    }

    /// Density at an arbitrary strike (linear between knots, 0 outside).
    pub fn q_at(&self, s: f64) -> f64 {
        q_interior(&self.u, &self.q, s)
    }
}

/// Raw moments ∫ sᵏ q ds, k = 0..4, of the piecewise-linear density over
/// knots i0..=i1: on [uᵢ, uᵢ₊₁] write q(s) = c₀ + c₁s; the integral of
/// sᵏ(c₀+c₁s) is exact.
pub fn raw_moments_interior(u: &[f64], gamma: &[f64], i0: usize, i1: usize) -> [f64; 5] {
    let mut raw = [0.0f64; 5];
    for i in i0..i1 {
        let h = u[i + 1] - u[i];
        let c1 = (gamma[i + 1] - gamma[i]) / h;
        let c0 = gamma[i] - c1 * u[i];
        let (a, b) = (u[i], u[i + 1]);
        for (kk, r) in raw.iter_mut().enumerate() {
            let p1 = (kk + 1) as f64;
            let p2 = (kk + 2) as f64;
            *r += c0 * (b.powf(p1) - a.powf(p1)) / p1 + c1 * (b.powf(p2) - a.powf(p2)) / p2;
        }
    }
    raw
}

/// Invert the piecewise-quadratic CDF given by knot densities `q` and
/// knot CDF values `cdf`. None if p falls outside the knot range.
pub fn quantile_interior(u: &[f64], q: &[f64], cdf: &[f64], p: f64) -> Option<f64> {
    let n = u.len();
    if p < cdf[0] || p > cdf[n - 1] {
        return None;
    }
    let i = cdf.partition_point(|&c| c <= p).min(n - 1);
    let i = i.saturating_sub(1);
    let (mut lo, mut hi) = (u[i], u[i + 1]);
    let h = hi - lo;
    let cdf_at = |s: f64| {
        let th = s - u[i];
        cdf[i] + q[i] * th + (q[i + 1] - q[i]) / (2.0 * h) * th * th
    };
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        if cdf_at(mid) < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some(0.5 * (lo + hi))
}

/// Piecewise-linear density evaluation on the knot grid (0 outside).
pub fn q_interior(u: &[f64], q: &[f64], s: f64) -> f64 {
    let n = u.len();
    if s < u[0] || s > u[n - 1] {
        return 0.0;
    }
    let i = u.partition_point(|&x| x <= s).min(n - 1);
    let i = i.saturating_sub(1);
    let th = (s - u[i]) / (u[i + 1] - u[i]);
    q[i] * (1.0 - th) + q[i + 1] * th
}

//! No-arbitrage pre-filter diagnostics on a discrete quote set.
//!
//! The complete single-maturity discrete no-arbitrage characterisation
//! (RND/methods/negative-densities.md): forward calls C̃ᵢ at strikes
//! K₁ < … < Kₙ are arbitrage-free iff (1) C̃ᵢ > 0, (2) call-spread slopes
//! lie in [-1, 0], (3) the piecewise-linear interpolant is convex. Plus
//! the one diagnostic to compute before anything else: the noise floor
//! √6·ε/ΔK² of the finite-difference density against its peak scale.

use crate::slice::SmileSlice;

#[derive(Clone, Debug)]
pub struct Diagnostics {
    pub n: usize,
    pub dk_min: f64,
    pub dk_median: f64,
    pub dk_max: f64,
    /// Quotes with C̃ᵢ ≤ 0.
    pub non_positive: usize,
    /// Call-spread slopes outside [-1, 0].
    pub slope_violations: usize,
    /// Worst slope excursion beyond the bounds (0 if none).
    pub worst_slope_excursion: f64,
    /// Adjacent slope decreases (butterfly arbitrage), sᵢ₊₁ < sᵢ.
    pub butterfly_violations: usize,
    /// Most negative slope increase (≥ 0 means clean).
    pub min_slope_increase: f64,
    /// √6·ε/ΔK²_median — s.d. of the finite-difference density under
    /// independent quote noise of s.d. ε.
    pub noise_floor: f64,
    /// Peak density scale 1/(F·σ_atm·√(2πt)) — lognormal ATM density.
    pub peak_density_scale: f64,
    /// Strike coverage in σ√t units: (left edge, right edge).
    pub coverage_sigma: (f64, f64),
    pub atm_vol: f64,
}

/// Run the diagnostics. `eps` is the quote-noise s.d. in price units —
/// half a tick is "the minimum irreducible uncertainty" (ICE Brent options
/// tick 0.01 $/bbl ⇒ eps = 0.005).
pub fn diagnose(slice: &SmileSlice, eps: f64) -> Diagnostics {
    let k = &slice.strikes;
    let c = &slice.calls;
    let n = k.len();

    let mut dks: Vec<f64> = k.windows(2).map(|w| w[1] - w[0]).collect();
    dks.sort_by(|a, b| a.total_cmp(b));
    let dk_min = dks.first().copied().unwrap_or(f64::NAN);
    let dk_max = dks.last().copied().unwrap_or(f64::NAN);
    let dk_median = if dks.is_empty() {
        f64::NAN
    } else {
        dks[dks.len() / 2]
    };

    let non_positive = c.iter().filter(|&&x| x <= 0.0).count();

    let slopes: Vec<f64> = (0..n.saturating_sub(1))
        .map(|i| (c[i + 1] - c[i]) / (k[i + 1] - k[i]))
        .collect();
    let mut slope_violations = 0;
    let mut worst_slope_excursion = 0.0f64;
    for &s in &slopes {
        let excursion = (s - 0.0).max(-1.0 - s).max(0.0);
        if excursion > 0.0 {
            slope_violations += 1;
            worst_slope_excursion = worst_slope_excursion.max(excursion);
        }
    }

    let mut butterfly_violations = 0;
    let mut min_slope_increase = f64::MAX;
    for w in slopes.windows(2) {
        let inc = w[1] - w[0];
        min_slope_increase = min_slope_increase.min(inc);
        if inc < 0.0 {
            butterfly_violations += 1;
        }
    }
    if slopes.len() < 2 {
        min_slope_increase = f64::NAN;
    }

    let atm_vol = slice.atm_vol();
    let sig_sqrt_t = atm_vol * slice.t.sqrt();
    let peak_density_scale = 1.0 / (slice.f * sig_sqrt_t * (2.0 * std::f64::consts::PI).sqrt());
    let noise_floor = 6.0f64.sqrt() * eps / (dk_median * dk_median);
    let coverage_sigma = (
        (k[0] / slice.f).ln() / sig_sqrt_t,
        (k[n - 1] / slice.f).ln() / sig_sqrt_t,
    );

    Diagnostics {
        n,
        dk_min,
        dk_median,
        dk_max,
        non_positive,
        slope_violations,
        worst_slope_excursion,
        butterfly_violations,
        min_slope_increase,
        noise_floor,
        peak_density_scale,
        coverage_sigma,
        atm_vol,
    }
}

impl Diagnostics {
    pub fn clean(&self) -> bool {
        self.non_positive == 0 && self.slope_violations == 0 && self.butterfly_violations == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slice::SmileSlice;

    fn slice_with(strikes: Vec<f64>, calls: Vec<f64>) -> SmileSlice {
        let n = strikes.len();
        SmileSlice {
            underlying: "TEST".into(),
            valuation_date: "2026-01-01".into(),
            f: 100.0,
            t: 0.25,
            strikes,
            vols: vec![0.3; n],
            calls,
        }
    }

    #[test]
    fn clean_convex_quotes_pass() {
        // Convex, decreasing, positive: a well-behaved call curve.
        let s = slice_with(
            vec![90.0, 95.0, 100.0, 105.0, 110.0],
            vec![12.0, 8.5, 5.8, 3.9, 2.6],
        );
        let d = diagnose(&s, 0.005);
        assert!(d.clean(), "{d:?}");
        assert!(d.min_slope_increase >= 0.0);
    }

    #[test]
    fn butterfly_violation_detected() {
        // Middle quote pushed up: slopes decrease between the last two
        // intervals -> exactly one butterfly violation.
        let s = slice_with(vec![90.0, 95.0, 100.0, 105.0], vec![12.0, 8.5, 7.5, 3.9]);
        let d = diagnose(&s, 0.005);
        assert_eq!(d.butterfly_violations, 1);
        assert!(!d.clean());
    }

    #[test]
    fn slope_bound_violation_detected() {
        // An increasing pair of quotes: slope > 0.
        let s = slice_with(vec![90.0, 95.0, 100.0], vec![10.0, 11.0, 5.0]);
        let d = diagnose(&s, 0.005);
        assert!(d.slope_violations >= 1);
    }
}

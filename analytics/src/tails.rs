//! GPD tail grafting — the Bollinger-Melick-Thomas repricing criterion.
//!
//! Per RND/methods/tail-extrapolation.md: GPD over GEV (two parameters,
//! same asymptotic ξ; the location is *not free* — it is the attachment
//! strike — and the tail mass constraint is satisfied exactly by scaling).
//! Instead of Figlewski's density value-matching (9a)-(9c), the shape is
//! chosen so the tail *reprices the options at the strikes it replaces*:
//!
//! - mass: the tail carries exactly the interior CDF's mass beyond the
//!   attachment knot (construction);
//! - C¹ price continuity at the splice: σ is pinned by the model option
//!   value at the attachment strike (the slope then matches automatically,
//!   because the slope is the tail mass);
//! - ξ: 1-D least squares repricing the observed quotes beyond attachment.
//!
//! Density continuity at the splice is deliberately NOT imposed — the jump
//! is reported as a diagnostic (the splice-discontinuity trap is real, but
//! value-matching is the weaker criterion; see the survey).
//!
//! Right tail, unconditional survival for s ≥ c:
//!   P(S > s) = p·z(s)^{-1/ξ},  z(s) = 1 + ξ(s-c)/σ
//! Forward call for K ≥ c:  C̃(K) = p·σ·z(K)^{1-1/ξ}/(1-ξ)   (ξ < 1)
//! Left tail mirrored on (c - S); forward puts via parity.

use crate::density::{self, Density};
use crate::fengler::FenglerFit;
use crate::slice::SmileSlice;
use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

#[derive(Clone, Debug)]
pub struct GpdTail {
    pub side: Side,
    /// Attachment strike c (a knot of the interior fit).
    pub attach: f64,
    /// Knot index of the attachment in the interior fit.
    pub attach_idx: usize,
    /// Unconditional tail mass p beyond the attachment.
    pub mass: f64,
    pub xi: f64,
    pub sigma: f64,
    /// RMSE of repricing the observed quotes beyond attachment.
    pub reprice_rmse: f64,
    pub n_reprice: usize,
    /// Density jump at the splice, normalized by the peak interior
    /// density: (q_tail(c) - γ_c)/max γ.
    pub density_jump_vs_peak: f64,
}

impl GpdTail {
    fn z(&self, s: f64) -> f64 {
        let x = match self.side {
            Side::Right => s - self.attach,
            Side::Left => self.attach - s,
        };
        1.0 + self.xi * x / self.sigma
    }

    /// Unconditional density at s (0 outside the tail's support).
    pub fn density(&self, s: f64) -> f64 {
        let z = self.z(s);
        if z <= 0.0 {
            return 0.0;
        }
        self.mass / self.sigma * z.powf(-1.0 / self.xi - 1.0)
    }

    /// Unconditional probability beyond s in the tail direction:
    /// P(S > s) (right) or P(S < s) (left).
    pub fn prob_beyond(&self, s: f64) -> f64 {
        let z = self.z(s);
        if z <= 0.0 {
            return 0.0;
        }
        self.mass * z.powf(-1.0 / self.xi)
    }

    /// Strike level at unconditional CDF level `p_level`. Callers must
    /// ensure the level falls inside this tail.
    pub fn quantile(&self, p_level: f64) -> f64 {
        let t = match self.side {
            Side::Right => (1.0 - p_level) / self.mass,
            Side::Left => p_level / self.mass,
        };
        let x = if self.xi.abs() < 1e-9 {
            -self.sigma * t.ln()
        } else {
            self.sigma / self.xi * (t.powf(-self.xi) - 1.0)
        };
        match self.side {
            Side::Right => self.attach + x,
            Side::Left => self.attach - x,
        }
    }

    /// Forward option value the tail assigns: a call at K ≥ c (right) or
    /// a put at K ≤ c (left).
    pub fn option_value(&self, k: f64) -> f64 {
        let z = self.z(k);
        if z <= 0.0 {
            return 0.0; // beyond a bounded (ξ<0) support endpoint
        }
        if self.xi.abs() < 1e-9 {
            return self.mass * self.sigma * (-(self.attach - k).abs() / self.sigma).exp();
        }
        self.mass * self.sigma * z.powf(1.0 - 1.0 / self.xi) / (1.0 - self.xi)
    }

    /// ∫ sᵏ q(s) ds over the tail. Infinite when ξ ≥ 1/k.
    pub fn raw_moment(&self, k: usize) -> f64 {
        // E[Xʲ] for X ~ GPD(σ,ξ): σʲ·j!/∏_{i=1..j}(1-iξ), finite iff ξ < 1/j.
        let mut ex = [0.0f64; 5];
        ex[0] = 1.0;
        for (j, e) in ex.iter_mut().enumerate().take(k + 1).skip(1) {
            if self.xi >= 1.0 / j as f64 {
                return f64::INFINITY;
            }
            let mut denom = 1.0;
            for i in 1..=j {
                denom *= 1.0 - i as f64 * self.xi;
            }
            *e = self.sigma.powi(j as i32) * factorial(j) / denom;
        }
        let c = self.attach;
        let sign: f64 = match self.side {
            Side::Right => 1.0,
            Side::Left => -1.0,
        };
        let mut sum = 0.0;
        for (j, &e) in ex.iter().enumerate().take(k + 1) {
            sum += binom(k, j) * c.powi((k - j) as i32) * sign.powi(j as i32) * e;
        }
        self.mass * sum
    }
}

fn factorial(n: usize) -> f64 {
    (1..=n).map(|i| i as f64).product()
}

fn binom(n: usize, k: usize) -> f64 {
    factorial(n) / (factorial(k) * factorial(n - k))
}

/// A tail fit needs at least this many informative quotes beyond the
/// attachment; the attachment moves inward from the α-quantile knot
/// until it has them.
const MIN_TARGETS: usize = 4;

/// Fit one tail. `alpha` is the target attachment quantile (the
/// attachment lands on a knot, so the actual mass — from the interior CDF
/// there — is reported in `mass`, and moves inward if the α-quantile sits
/// off-grid or too few informative quotes lie beyond). `min_premium`
/// drops tick-floor quotes from the repricing targets — deep-wing settles
/// pinned at the minimum tick carry no density information and bias ξ.
fn fit_tail(
    side: Side,
    slice: &SmileSlice,
    fit: &FenglerFit,
    dens: &Density,
    alpha: f64,
    min_premium: f64,
) -> Result<GpdTail> {
    let u = &fit.u;
    let n = u.len();

    // OTM option value per knot (puts below the forward via parity) and
    // whether it is informative (above the tick floor).
    let otm_value = |i: usize| -> f64 {
        match side {
            Side::Right => slice.calls[i],
            Side::Left => slice.calls[i] - (slice.f - u[i]),
        }
    };
    let informative = |i: usize| otm_value(i) >= min_premium;

    // Attachment knot: start at the CDF crossing of the α-quantile, then
    // move inward until MIN_TARGETS informative quotes lie beyond. Never
    // cross the 25% quantile — a "tail" replacing the smile's shoulder
    // would be answering a different question.
    let (j, p) = match side {
        Side::Right => {
            let mut j = dens
                .cdf
                .iter()
                .position(|&c| c >= 1.0 - alpha)
                .unwrap_or(n - 1);
            let limit = dens.cdf.iter().position(|&c| c >= 0.75).unwrap_or(0);
            while j > limit && (j + 1..n).filter(|&i| informative(i)).count() < MIN_TARGETS {
                j -= 1;
            }
            (j, 1.0 - dens.cdf[j])
        }
        Side::Left => {
            let mut j = dens.cdf.iter().rposition(|&c| c <= alpha).unwrap_or(0);
            let limit = dens.cdf.iter().rposition(|&c| c <= 0.25).unwrap_or(n - 1);
            while j < limit && (0..j).filter(|&i| informative(i)).count() < MIN_TARGETS {
                j += 1;
            }
            (j, dens.cdf[j])
        }
    };
    let targets: Vec<(f64, f64)> = match side {
        Side::Right => (j + 1..n)
            .filter(|&i| informative(i))
            .map(|i| (u[i], otm_value(i)))
            .collect(),
        Side::Left => (0..j)
            .filter(|&i| informative(i))
            .map(|i| (u[i], otm_value(i)))
            .collect(),
    };
    if targets.len() < 2 {
        return Err(Error::Solver(format!(
            "{side:?} tail: only {} informative quotes beyond any admissible attachment",
            targets.len()
        )));
    }
    if p <= 0.0 {
        return Err(Error::Solver(format!(
            "{side:?} tail: no mass beyond attachment knot {j} (CDF degenerate there)"
        )));
    }
    let c = u[j];

    // Model option value at the attachment (forward space): pins σ given ξ
    // so the stitched price curve is C¹ at the splice.
    let value_at_c = match side {
        Side::Right => fit.g[j],
        Side::Left => fit.g[j] - (slice.f - c), // put via parity
    };
    if value_at_c <= 0.0 {
        return Err(Error::Solver(format!(
            "{side:?} tail: non-positive model option value {value_at_c:.2e} at attachment {c}"
        )));
    }

    let tail_for = |xi: f64| -> Option<GpdTail> {
        let sigma = value_at_c * (1.0 - xi) / p;
        if sigma <= 0.0 || !sigma.is_finite() {
            return None;
        }
        Some(GpdTail {
            side,
            attach: c,
            attach_idx: j,
            mass: p,
            xi,
            sigma,
            reprice_rmse: 0.0,
            n_reprice: targets.len(),
            density_jump_vs_peak: 0.0,
        })
    };
    let sse = |xi: f64| -> f64 {
        match tail_for(xi) {
            None => f64::INFINITY,
            Some(t) => targets
                .iter()
                .map(|&(k, y)| (t.option_value(k) - y).powi(2))
                .sum(),
        }
    };

    // Coarse grid over ξ ∈ [-3, 0.95), then golden-section refine. ξ ≥ 1
    // has no finite mean; the grid floor covers sharply bounded supports.
    let grid: Vec<f64> = (0..160).map(|i| -3.0 + i as f64 * 0.0248).collect();
    let (mut best_i, mut best) = (0, f64::INFINITY);
    for (i, &xi) in grid.iter().enumerate() {
        let v = sse(xi);
        if v < best {
            best = v;
            best_i = i;
        }
    }
    if !best.is_finite() {
        return Err(Error::Solver(format!("{side:?} tail: no feasible ξ")));
    }
    let (mut a, mut b) = (
        grid[best_i.saturating_sub(1)],
        grid[(best_i + 1).min(grid.len() - 1)],
    );
    const PHI: f64 = 0.618_033_988_749_895;
    for _ in 0..80 {
        let x1 = b - PHI * (b - a);
        let x2 = a + PHI * (b - a);
        if sse(x1) < sse(x2) {
            b = x2;
        } else {
            a = x1;
        }
    }
    let xi = 0.5 * (a + b);

    let mut tail = tail_for(xi)
        .ok_or_else(|| Error::Solver(format!("{side:?} tail: infeasible fitted ξ {xi}")))?;
    tail.reprice_rmse = (sse(xi) / targets.len().max(1) as f64).sqrt();
    let gamma_peak = fit.gamma.iter().copied().fold(0.0f64, f64::max);
    tail.density_jump_vs_peak = (tail.density(c) - fit.gamma[j]) / gamma_peak;
    Ok(tail)
}

/// The stitched density: GPD tails outside the attachment knots, the
/// Fengler interior between them. Mass sums to 1 by construction.
#[derive(Clone, Debug)]
pub struct Stitched {
    pub left: GpdTail,
    pub right: GpdTail,
    /// Interior knots between the attachments (inclusive).
    pub u: Vec<f64>,
    pub q: Vec<f64>,
    pub cdf: Vec<f64>,
    pub mean: f64,
    pub sd: f64,
    pub skew: f64,
    pub kurt: f64,
    /// Mass the left tail puts below S = 0 (report if non-negligible;
    /// a GPD left tail with ξ ≥ 0 is unbounded below).
    pub mass_below_zero: f64,
}

pub fn graft(
    slice: &SmileSlice,
    fit: &FenglerFit,
    dens: &Density,
    alpha: f64,
    min_premium: f64,
) -> Result<Stitched> {
    let left = fit_tail(Side::Left, slice, fit, dens, alpha, min_premium)?;
    let right = fit_tail(Side::Right, slice, fit, dens, alpha, min_premium)?;
    let (jl, jr) = (left.attach_idx, right.attach_idx);
    if jl >= jr {
        return Err(Error::Solver(format!(
            "attachments cross: left knot {jl} ≥ right knot {jr} (alpha {alpha} too large?)"
        )));
    }

    let interior = density::raw_moments_interior(&fit.u, &fit.gamma, jl, jr);
    let mut raw = [0.0f64; 5];
    for (k, r) in raw.iter_mut().enumerate() {
        *r = interior[k] + left.raw_moment(k) + right.raw_moment(k);
    }
    let mean = raw[1];
    let var = (raw[2] - mean * mean).max(0.0);
    let sd = var.sqrt();
    let m3 = raw[3] - 3.0 * mean * raw[2] + 2.0 * mean.powi(3);
    let m4 = raw[4] - 4.0 * mean * raw[3] + 6.0 * mean * mean * raw[2] - 3.0 * mean.powi(4);
    let skew = if sd > 0.0 { m3 / sd.powi(3) } else { f64::NAN };
    let kurt = if sd > 0.0 { m4 / sd.powi(4) } else { f64::NAN };

    Ok(Stitched {
        u: fit.u[jl..=jr].to_vec(),
        q: fit.gamma[jl..=jr].to_vec(),
        cdf: dens.cdf[jl..=jr].to_vec(),
        mass_below_zero: left.prob_beyond(0.0),
        left,
        right,
        mean,
        sd,
        skew,
        kurt,
    })
}

impl Stitched {
    /// Quantile of the stitched distribution at CDF level p.
    pub fn quantile(&self, p: f64) -> Option<f64> {
        if !(0.0..=1.0).contains(&p) {
            return None;
        }
        if p < self.cdf[0] {
            return Some(self.left.quantile(p));
        }
        if p > self.cdf[self.cdf.len() - 1] {
            return Some(self.right.quantile(p));
        }
        density::quantile_interior(&self.u, &self.q, &self.cdf, p)
    }

    /// Density at any strike: interior piecewise linear, GPD outside.
    pub fn q_at(&self, s: f64) -> f64 {
        if s < self.u[0] {
            return self.left.density(s);
        }
        if s > self.u[self.u.len() - 1] {
            return self.right.density(s);
        }
        density::q_interior(&self.u, &self.q, s)
    }

    /// CDF at any strike.
    pub fn cdf_at(&self, s: f64) -> f64 {
        if s < self.u[0] {
            return self.left.prob_beyond(s);
        }
        let n = self.u.len();
        if s > self.u[n - 1] {
            return 1.0 - self.right.prob_beyond(s);
        }
        let i = self.u.partition_point(|&x| x <= s).min(n - 1).max(1) - 1;
        let h = self.u[i + 1] - self.u[i];
        let th = s - self.u[i];
        self.cdf[i] + self.q[i] * th + (self.q[i + 1] - self.q[i]) / (2.0 * h) * th * th
    }
}

/// Lee's moment-formula cross-check: wing slopes of total implied
/// variance w(k) = σ²t in log-moneyness map to moment bounds (p̃, q̃),
/// and the right slope to a tail index the fitted ξ must agree with.
#[derive(Clone, Debug)]
pub struct LeeCheck {
    /// d w / d|k| on the outermost nodes, per side. Lee: β ∈ [0, 2].
    pub beta_left: f64,
    pub beta_right: f64,
    /// sup{p : E[S^{1+p}] < ∞} implied by β_right (∞ for β = 0).
    pub p_tilde: f64,
    /// sup{q : E[S^{-q}] < ∞} implied by β_left.
    pub q_tilde: f64,
    /// GPD shape the right wing implies: ξ = 1/(1+p̃).
    pub xi_lee_right: f64,
    pub n_wing: usize,
}

/// `min_premium` excludes tick-floor nodes from the wing regressions —
/// their re-implied vols reflect quote quantization, not the smile.
pub fn lee_check(slice: &SmileSlice, n_wing: usize, min_premium: f64) -> LeeCheck {
    let n = slice.strikes.len();
    let w: Vec<f64> = slice.vols.iter().map(|v| v * v * slice.t).collect();
    let k: Vec<f64> = slice.strikes.iter().map(|s| (s / slice.f).ln()).collect();
    let otm: Vec<f64> = (0..n)
        .map(|i| {
            if k[i] >= 0.0 {
                slice.calls[i]
            } else {
                slice.calls[i] - (slice.f - slice.strikes[i])
            }
        })
        .collect();

    // OLS slope of w on |k| over the outermost m informative nodes.
    let slope = |wing: &[usize]| -> f64 {
        let xs: Vec<f64> = wing.iter().map(|&i| k[i].abs()).collect();
        let ys: Vec<f64> = wing.iter().map(|&i| w[i]).collect();
        let nn = xs.len() as f64;
        let (mx, my) = (xs.iter().sum::<f64>() / nn, ys.iter().sum::<f64>() / nn);
        let sxy: f64 = xs.iter().zip(&ys).map(|(x, y)| (x - mx) * (y - my)).sum();
        let sxx: f64 = xs.iter().map(|x| (x - mx) * (x - mx)).sum();
        if sxx > 0.0 { sxy / sxx } else { f64::NAN }
    };
    let informative: Vec<usize> = (0..n).filter(|&i| otm[i] >= min_premium).collect();
    let m = n_wing.clamp(3, informative.len().max(6) / 2);
    let left_wing: Vec<usize> = informative.iter().copied().take(m).collect();
    let right_wing: Vec<usize> = informative.iter().copied().rev().take(m).rev().collect();
    let beta_left = slope(&left_wing);
    let beta_right = slope(&right_wing);

    let moment_bound = |beta: f64| -> f64 {
        if beta <= 0.0 {
            f64::INFINITY
        } else {
            1.0 / (2.0 * beta) + beta / 8.0 - 0.5
        }
    };
    let p_tilde = moment_bound(beta_right);
    let q_tilde = moment_bound(beta_left);
    let xi_lee_right = if p_tilde.is_infinite() {
        0.0
    } else {
        1.0 / (1.0 + p_tilde)
    };

    LeeCheck {
        beta_left,
        beta_right,
        p_tilde,
        q_tilde,
        xi_lee_right,
        n_wing: m,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tail(side: Side, xi: f64) -> GpdTail {
        GpdTail {
            side,
            attach: 100.0,
            attach_idx: 0,
            mass: 0.05,
            xi,
            sigma: 4.0,
            reprice_rmse: 0.0,
            n_reprice: 0,
            density_jump_vs_peak: 0.0,
        }
    }

    /// Numeric integration of the tail density must reproduce the mass,
    /// first raw moment, and option value formulas.
    #[test]
    fn gpd_formulas_match_numeric_integrals() {
        for &(side, xi) in &[
            (Side::Right, 0.2),
            (Side::Right, -0.3),
            (Side::Right, 1e-12),
            (Side::Left, 0.15),
            (Side::Left, -0.4),
        ] {
            let t = tail(side, xi);
            // Integrate to the 1e-9 quantile (or the bounded endpoint).
            let far = match side {
                Side::Right => t.quantile(1.0 - 1e-9 * t.mass),
                Side::Left => t.quantile(1e-9 * t.mass),
            };
            let (a, b) = match side {
                Side::Right => (t.attach, far),
                Side::Left => (far, t.attach),
            };
            let n = 400_000;
            let h = (b - a) / n as f64;
            let (mut mass, mut m1, mut call) = (0.0, 0.0, 0.0);
            let k_test = match side {
                Side::Right => 103.0,
                Side::Left => 97.0,
            };
            for i in 0..n {
                let s = a + (i as f64 + 0.5) * h;
                let q = t.density(s);
                mass += q * h;
                m1 += s * q * h;
                call += match side {
                    Side::Right => (s - k_test).max(0.0) * q * h,
                    Side::Left => (k_test - s).max(0.0) * q * h,
                };
            }
            assert!(
                (mass - t.mass).abs() < 1e-5,
                "{side:?} ξ={xi}: mass {mass} vs {}",
                t.mass
            );
            assert!(
                (m1 - t.raw_moment(1)).abs() < 1e-3,
                "{side:?} ξ={xi}: m1 {m1} vs {}",
                t.raw_moment(1)
            );
            assert!(
                (call - t.option_value(k_test)).abs() < 1e-5,
                "{side:?} ξ={xi}: value {call} vs {}",
                t.option_value(k_test)
            );
        }
    }

    #[test]
    fn quantile_roundtrips_through_cdf() {
        for &(side, xi) in &[(Side::Right, 0.25), (Side::Left, -0.2)] {
            let t = tail(side, xi);
            for &lvl in &[0.001, 0.01, 0.03] {
                let p_level = match side {
                    Side::Right => 1.0 - lvl,
                    Side::Left => lvl,
                };
                let s = t.quantile(p_level);
                let back = t.prob_beyond(s);
                let expect = match side {
                    Side::Right => 1.0 - p_level,
                    Side::Left => p_level,
                };
                assert!(
                    (back - expect).abs() < 1e-12,
                    "{side:?} ξ={xi} level {p_level}: {back} vs {expect}"
                );
            }
        }
    }

    #[test]
    fn divergent_moments_are_flagged() {
        let t = tail(Side::Right, 0.6); // ξ ≥ 1/2: variance infinite
        assert!(t.raw_moment(1).is_finite());
        assert!(t.raw_moment(2).is_infinite());
    }
}

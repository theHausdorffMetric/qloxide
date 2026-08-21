//! The Bliss-Panigirtzoglou perturbation test, as a harness.
//!
//! Perturb every quote by a uniform draw in ±ε (half a tick — "the
//! minimum irreducible uncertainty associated with option prices"),
//! re-run the entire pipeline (Fengler QP → Breeden-Litzenberger → GPD
//! graft), repeat N times, and report the empirical distribution of every
//! summary statistic. This is a lower bound on the confidence interval of
//! every number the pipeline publishes, and — since no condition-number
//! theory exists for these estimators — the only general instability
//! diagnostic available (RND/methods/negative-densities.md).

use crate::fengler::{self, FenglerConfig};
use crate::slice::SmileSlice;
use crate::{Result, density, tails};

/// One pipeline run's published numbers.
#[derive(Clone, Debug)]
pub struct DayStats {
    pub mean: f64,
    pub sd: f64,
    pub skew: f64,
    pub kurt: f64,
    /// Quantiles at [p01, p05, p10, p25, p50, p75, p90, p95, p99].
    pub quantiles: [f64; 9],
    pub xi_left: f64,
    pub xi_right: f64,
    pub max_fit_err: f64,
    pub min_gamma: f64,
}

pub const QUANTILE_LEVELS: [f64; 9] = [0.01, 0.05, 0.10, 0.25, 0.50, 0.75, 0.90, 0.95, 0.99];

/// Run the full pipeline once: the stat vector plus the stitched density
/// it was read from (kept for plotting and envelope evaluation).
pub fn pipeline(
    slice: &SmileSlice,
    cfg: &FenglerConfig,
    alpha: f64,
    min_premium: f64,
) -> Result<(DayStats, tails::Stitched)> {
    let fit = fengler::fit(&slice.strikes, &slice.calls, slice.f, cfg)?;
    let dens = density::extract(&fit);
    let st = tails::graft(slice, &fit, &dens, alpha, min_premium)?;
    let mut quantiles = [f64::NAN; 9];
    for (q, &lvl) in quantiles.iter_mut().zip(&QUANTILE_LEVELS) {
        *q = st.quantile(lvl).unwrap_or(f64::NAN);
    }
    let stats = DayStats {
        mean: st.mean,
        sd: st.sd,
        skew: st.skew,
        kurt: st.kurt,
        quantiles,
        xi_left: st.left.xi,
        xi_right: st.right.xi,
        max_fit_err: fit.max_fit_err,
        min_gamma: fit.min_gamma,
    };
    Ok((stats, st))
}

/// Run the full pipeline once and collect the stat vector.
pub fn day_stats(
    slice: &SmileSlice,
    cfg: &FenglerConfig,
    alpha: f64,
    min_premium: f64,
) -> Result<DayStats> {
    pipeline(slice, cfg, alpha, min_premium).map(|(s, _)| s)
}

#[derive(Clone, Debug)]
pub struct PerturbConfig {
    /// Number of perturbation draws (Norges Bank's variant uses 500).
    pub n: usize,
    /// Uniform perturbation half-width in price units (half a tick).
    pub eps: f64,
    pub seed: u64,
    pub alpha: f64,
    pub min_premium: f64,
    pub fengler: FenglerConfig,
    /// When > 0, also evaluate each draw's density on a fixed grid of
    /// this many points and keep pointwise 5%/50%/95% — the plotted
    /// perturbation envelope. 0 skips it.
    pub envelope_points: usize,
}

/// Pointwise density band across the draws, on a fixed strike grid
/// spanning the baseline's 0.1%..99.9% quantile range.
#[derive(Clone, Debug)]
pub struct Envelope {
    pub grid: Vec<f64>,
    pub p05: Vec<f64>,
    pub median: Vec<f64>,
    pub p95: Vec<f64>,
}

/// Empirical distribution of one statistic across the draws.
#[derive(Clone, Debug)]
pub struct StatBand {
    pub name: &'static str,
    pub baseline: f64,
    pub p05: f64,
    pub median: f64,
    pub p95: f64,
    /// Draws where the statistic was non-finite (e.g. kurtosis with a
    /// fitted ξ ≥ ¼). Non-finite values sort into the upper percentiles
    /// rather than being dropped.
    pub non_finite: usize,
}

#[derive(Clone, Debug)]
pub struct PerturbReport {
    pub n_ok: usize,
    pub n_failed: usize,
    pub baseline: DayStats,
    pub bands: Vec<StatBand>,
    pub envelope: Option<Envelope>,
}

/// xorshift64* — deterministic, dependency-free; plenty for a
/// perturbation harness (this is not cryptography).
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.max(1))
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn uniform(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

pub fn run(slice: &SmileSlice, cfg: &PerturbConfig) -> Result<PerturbReport> {
    let (baseline, baseline_st) = pipeline(slice, &cfg.fengler, cfg.alpha, cfg.min_premium)?;

    // Envelope grid spans the baseline's 0.1%..99.9% quantile range.
    let grid: Vec<f64> = if cfg.envelope_points > 0 {
        let lo = baseline_st.quantile(0.001).unwrap_or(slice.strikes[0]);
        let hi = baseline_st
            .quantile(0.999)
            .unwrap_or(*slice.strikes.last().unwrap());
        let m = cfg.envelope_points.max(2);
        (0..m)
            .map(|i| lo + (hi - lo) * i as f64 / (m - 1) as f64)
            .collect()
    } else {
        Vec::new()
    };

    let mut rng = Rng::new(cfg.seed);
    let mut draws: Vec<DayStats> = Vec::with_capacity(cfg.n);
    let mut density_rows: Vec<Vec<f64>> = Vec::new();
    let mut n_failed = 0;
    for _ in 0..cfg.n {
        let mut p = slice.clone();
        for c in &mut p.calls {
            *c = (*c + rng.uniform(-cfg.eps, cfg.eps)).max(0.0);
        }
        match pipeline(&p, &cfg.fengler, cfg.alpha, cfg.min_premium) {
            Ok((s, st)) => {
                if !grid.is_empty() {
                    density_rows.push(grid.iter().map(|&x| st.q_at(x)).collect());
                }
                draws.push(s);
            }
            Err(_) => n_failed += 1,
        }
    }

    let envelope = if grid.is_empty() || density_rows.is_empty() {
        None
    } else {
        let pointwise = |p: f64| -> Vec<f64> {
            (0..grid.len())
                .map(|i| {
                    let mut col: Vec<f64> = density_rows.iter().map(|r| r[i]).collect();
                    col.sort_by(|a, b| a.total_cmp(b));
                    col[((col.len() - 1) as f64 * p).round() as usize]
                })
                .collect()
        };
        let (p05, median, p95) = (pointwise(0.05), pointwise(0.50), pointwise(0.95));
        Some(Envelope {
            grid,
            p05,
            median,
            p95,
        })
    };

    let mut bands = Vec::new();
    let mut push = |name: &'static str, baseline: f64, values: Vec<f64>| {
        let mut v = values;
        v.sort_by(|a, b| a.total_cmp(b));
        let pick = |p: f64| {
            if v.is_empty() {
                f64::NAN
            } else {
                v[((v.len() - 1) as f64 * p).round() as usize]
            }
        };
        bands.push(StatBand {
            name,
            baseline,
            p05: pick(0.05),
            median: pick(0.50),
            p95: pick(0.95),
            non_finite: v.iter().filter(|x| !x.is_finite()).count(),
        });
    };

    push(
        "mean",
        baseline.mean,
        draws.iter().map(|s| s.mean).collect(),
    );
    push("sd", baseline.sd, draws.iter().map(|s| s.sd).collect());
    push(
        "skew",
        baseline.skew,
        draws.iter().map(|s| s.skew).collect(),
    );
    push(
        "kurt",
        baseline.kurt,
        draws.iter().map(|s| s.kurt).collect(),
    );
    let names = [
        "p01", "p05", "p10", "p25", "p50", "p75", "p90", "p95", "p99",
    ];
    for (i, name) in names.into_iter().enumerate() {
        push(
            name,
            baseline.quantiles[i],
            draws.iter().map(|s| s.quantiles[i]).collect(),
        );
    }
    push(
        "xi_left",
        baseline.xi_left,
        draws.iter().map(|s| s.xi_left).collect(),
    );
    push(
        "xi_right",
        baseline.xi_right,
        draws.iter().map(|s| s.xi_right).collect(),
    );

    Ok(PerturbReport {
        n_ok: draws.len(),
        n_failed,
        baseline,
        bands,
        envelope,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qloxide::instruments::PutOrCall;
    use qloxide::pricing::black76::{Black76Params, black76_price};

    fn lognormal_slice() -> SmileSlice {
        let (f, t, sigma) = (100.0f64, 0.25f64, 0.25f64);
        let strikes: Vec<f64> = (60..=150).map(|k| k as f64).collect();
        let calls: Vec<f64> = strikes
            .iter()
            .map(|&k| {
                black76_price(
                    Black76Params {
                        f,
                        k,
                        t,
                        r: 0.0,
                        sigma,
                    },
                    PutOrCall::Call,
                )
                .unwrap()
            })
            .collect();
        SmileSlice {
            underlying: "TEST".into(),
            valuation_date: "2026-01-01".into(),
            f,
            t,
            strikes: strikes.clone(),
            vols: vec![sigma; strikes.len()],
            calls,
        }
    }

    #[test]
    fn perturbation_bands_bracket_the_truth() {
        let slice = lognormal_slice();
        let cfg = PerturbConfig {
            n: 15,
            eps: 0.005,
            seed: 42,
            alpha: 0.05,
            min_premium: 0.02,
            fengler: FenglerConfig {
                lambda: 1e-4,
                weights: None,
            },
            envelope_points: 50,
        };
        let rep = run(&slice, &cfg).unwrap();
        assert_eq!(rep.n_failed, 0, "refits failed under half-tick noise");
        let env = rep.envelope.as_ref().expect("envelope requested");
        assert_eq!(env.grid.len(), 50);
        assert!(
            env.p05
                .iter()
                .zip(&env.p95)
                .all(|(lo, hi)| lo <= hi && *lo >= 0.0),
            "envelope bounds disordered"
        );

        // The martingale mean is structural: the band must be degenerate
        // at F to numerical precision.
        let mean = rep.bands.iter().find(|b| b.name == "mean").unwrap();
        assert!((mean.p05 - 100.0).abs() < 1e-6 && (mean.p95 - 100.0).abs() < 1e-6);

        // The sd band brackets the analytic lognormal sd and is tight.
        let sd_truth = 100.0 * ((0.25f64 * 0.25 * 0.25).exp() - 1.0).sqrt();
        let sd = rep.bands.iter().find(|b| b.name == "sd").unwrap();
        assert!(
            sd.p05 <= sd_truth && sd_truth <= sd.p95,
            "sd band [{}, {}] misses {sd_truth}",
            sd.p05,
            sd.p95
        );
        assert!(
            (sd.p95 - sd.p05) / sd_truth < 0.05,
            "sd band implausibly wide"
        );
    }

    #[test]
    fn same_seed_reproduces() {
        let slice = lognormal_slice();
        let cfg = PerturbConfig {
            n: 5,
            eps: 0.005,
            seed: 7,
            alpha: 0.05,
            min_premium: 0.02,
            fengler: FenglerConfig {
                lambda: 1e-4,
                weights: None,
            },
            envelope_points: 0,
        };
        let a = run(&slice, &cfg).unwrap();
        let b = run(&slice, &cfg).unwrap();
        for (x, y) in a.bands.iter().zip(&b.bands) {
            assert_eq!(x.median.to_bits(), y.median.to_bits(), "{}", x.name);
        }
    }
}

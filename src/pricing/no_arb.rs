//! Static no-arbitrage gate for vol surfaces.
//!
//! Model marks for uncleared positions are books-grade P&L, so the surface
//! that produces them must be free of static arbitrage — a butterfly
//! violation in the surface is a mispriced density baked straight into the
//! official books. This gate is the entry requirement (architecture.md §5):
//! it runs at portfolio load for every surface that will mark an uncleared
//! position, and findings are integrity errors, not warnings.
//!
//! Checks are performed on undiscounted Black-76 call prices at F = 1 (the
//! grid's moneyness axis is relative, so the forward level cancels):
//!
//! - **slope**: dC/dK within [-1, 0] between adjacent strikes;
//! - **butterfly**: call prices convex across each strike triple;
//! - **calendar**: total variance σ²·t non-decreasing in tenor at fixed
//!   moneyness.
//!
//! The tolerance is deliberately tight (float noise, not ticks): re-implied
//! settle surfaces carry genuine tick-level violations on real days — for a
//! cleared book that is a risk-side diagnostic, but for books-grade model
//! marks the alarm is the deliverable. Repairing the surface is a policy
//! decision upstream (the driver), never a silent fix here.

use crate::instruments::PutOrCall;
use crate::market_data::VolSurface;
use crate::pricing::black76::{Black76Params, black76_price};

/// Absolute tolerance on price-space comparisons at F = 1. Wide enough for
/// interpolation float noise, far below tick scale (a 0.01 tick on a ~70
/// forward is ~1.4e-4 at F = 1).
const TOL: f64 = 1e-7;

/// Check a surface for static arbitrage. Returns one finding per violation
/// (empty = clean). `Flat` surfaces are trivially clean.
pub fn surface_no_arb(surface: &VolSurface) -> Vec<String> {
    let VolSurface::Grid {
        tenors,
        moneyness,
        vols,
    } = surface
    else {
        return Vec::new();
    };

    let mut findings = Vec::new();
    let strikes: Vec<f64> = moneyness.iter().map(|m| m.exp()).collect();

    for (i, t) in tenors.iter().enumerate() {
        let prices: Vec<f64> = strikes
            .iter()
            .zip(&vols[i])
            .map(|(&k, &sigma)| {
                black76_price(
                    Black76Params {
                        f: 1.0,
                        k,
                        t: *t,
                        r: 0.0,
                        sigma,
                    },
                    PutOrCall::Call,
                )
                .unwrap_or(f64::NAN)
            })
            .collect();

        for j in 0..prices.len().saturating_sub(1) {
            let slope = (prices[j + 1] - prices[j]) / (strikes[j + 1] - strikes[j]);
            if !(-1.0 - TOL..=TOL).contains(&slope) {
                findings.push(format!(
                    "tenor {t}: call slope {slope:.3e} outside [-1, 0] between moneyness {} and {}",
                    moneyness[j],
                    moneyness[j + 1],
                ));
            }
        }
        for j in 1..prices.len().saturating_sub(1) {
            let left = (prices[j] - prices[j - 1]) / (strikes[j] - strikes[j - 1]);
            let right = (prices[j + 1] - prices[j]) / (strikes[j + 1] - strikes[j]);
            if right < left - TOL {
                findings.push(format!(
                    "tenor {t}: butterfly violation (concavity {:.3e}) at moneyness {}",
                    left - right,
                    moneyness[j],
                ));
            }
        }
    }

    for j in 0..moneyness.len() {
        for i in 1..tenors.len() {
            let w_prev = vols[i - 1][j] * vols[i - 1][j] * tenors[i - 1];
            let w = vols[i][j] * vols[i][j] * tenors[i];
            if w < w_prev - TOL {
                findings.push(format!(
                    "calendar violation at moneyness {}: total variance falls {:.3e} from tenor {} to {}",
                    moneyness[j],
                    w_prev - w,
                    tenors[i - 1],
                    tenors[i],
                ));
            }
        }
    }

    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(tenors: Vec<f64>, moneyness: Vec<f64>, vols: Vec<Vec<f64>>) -> VolSurface {
        VolSurface::Grid {
            tenors,
            moneyness,
            vols,
        }
    }

    #[test]
    fn flat_surface_is_clean() {
        assert!(surface_no_arb(&VolSurface::Flat { vol: 0.3 }).is_empty());
    }

    #[test]
    fn smooth_smile_is_clean() {
        // Gentle symmetric smile, two tenors with growing total variance.
        let m = vec![-0.2, -0.1, 0.0, 0.1, 0.2];
        let smile = |base: f64| m.iter().map(|k| base + 0.5 * k * k).collect::<Vec<_>>();
        let g = grid(vec![0.25, 0.5], m.clone(), vec![smile(0.30), smile(0.31)]);
        assert_eq!(surface_no_arb(&g), Vec::<String>::new());
    }

    #[test]
    fn butterfly_violation_detected() {
        // A vol spike at one strike makes the call-price curve concave there.
        let m = vec![-0.2, -0.1, 0.0, 0.1, 0.2];
        let vols = vec![vec![0.30, 0.30, 0.60, 0.30, 0.30]];
        let g = grid(vec![0.25], m, vols);
        let findings = surface_no_arb(&g);
        assert!(
            findings.iter().any(|f| f.contains("butterfly")),
            "expected a butterfly finding, got: {findings:?}"
        );
    }

    #[test]
    fn calendar_violation_detected() {
        // Total variance shrinking with tenor at fixed moneyness.
        let m = vec![-0.1, 0.0, 0.1];
        let vols = vec![vec![0.40, 0.40, 0.40], vec![0.20, 0.20, 0.20]];
        let g = grid(vec![0.25, 0.5], m, vols);
        let findings = surface_no_arb(&g);
        assert!(
            findings.iter().any(|f| f.contains("calendar")),
            "expected a calendar finding, got: {findings:?}"
        );
    }
}

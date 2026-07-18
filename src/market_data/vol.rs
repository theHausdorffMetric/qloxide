use serde::{Deserialize, Serialize};

use crate::core;

/// Volatility surface — vol as a function of tenor and moneyness.
///
/// `tenor` is time to expiry in years; `moneyness` is log-moneyness
/// `ln(K/F)`. The surface variant also carries the model: a `Flat` or
/// `Grid` (lognormal) vol implies Black76. A future `FlatBachelier`
/// (normal vol) variant will imply Bachelier — no separate model
/// parameter exists.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum VolSurface {
    /// Constant lognormal vol regardless of tenor/moneyness.
    Flat { vol: f64 },
    /// Lognormal vols on a tenor × log-moneyness grid.
    ///
    /// Interpolation: **linear in vol** across moneyness (within a tenor
    /// row), **linear in total variance** σ²·t across tenors, flat
    /// extrapolation beyond either axis. `vols[i][j]` is the vol at
    /// `tenors[i]`, `moneyness[j]`; both axes strictly increasing.
    Grid {
        tenors: Vec<f64>,
        moneyness: Vec<f64>,
        vols: Vec<Vec<f64>>,
    },
    // Future variants:
    // FlatBachelier { vol } — normal model
    // Sabr { alpha, beta, rho, nu } — parametric
}

/// Linear interpolation of `ys` over strictly-increasing `xs` at `x`,
/// flat (clamped) extrapolation outside the range. `xs` must be
/// non-empty and `ys` the same length — guaranteed by [`VolSurface::validate`].
fn interp_clamped(xs: &[f64], ys: &[f64], x: f64) -> f64 {
    debug_assert_eq!(xs.len(), ys.len());
    debug_assert!(!xs.is_empty());
    if x <= xs[0] {
        return ys[0];
    }
    if x >= xs[xs.len() - 1] {
        return ys[ys.len() - 1];
    }
    // partition_point: first index with xs[i] > x; x is strictly inside
    // the range here, so 1 <= i <= len-1.
    let i = xs.partition_point(|&p| p <= x);
    let (x0, x1) = (xs[i - 1], xs[i]);
    let (y0, y1) = (ys[i - 1], ys[i]);
    let w = (x - x0) / (x1 - x0);
    y0 + w * (y1 - y0)
}

impl VolSurface {
    /// Volatility at the given tenor (years) and log-moneyness ln(K/F).
    ///
    /// Total for surfaces that pass [`validate`](Self::validate); call
    /// that after deserializing untrusted data (the config loader does).
    pub fn vol(&self, tenor: f64, moneyness: f64) -> f64 {
        match self {
            VolSurface::Flat { vol } => *vol,
            VolSurface::Grid {
                tenors,
                moneyness: ms,
                vols,
            } => {
                let last = tenors.len() - 1;
                // Row lookups interpolate the smile in vol at fixed tenor.
                let row = |i: usize| interp_clamped(ms, &vols[i], moneyness);
                if tenor <= tenors[0] {
                    return row(0); // flat extrapolation, short end
                }
                if tenor >= tenors[last] {
                    return row(last); // flat extrapolation, long end
                }
                let i = tenors.partition_point(|&p| p <= tenor);
                let (t0, t1) = (tenors[i - 1], tenors[i]);
                let (v0, v1) = (row(i - 1), row(i));
                // Linear in total variance σ²·t between the bracketing
                // tenors; both endpoints are ≥ 0, so the interpolant is.
                let w = (tenor - t0) / (t1 - t0);
                let var = v0 * v0 * t0 + w * (v1 * v1 * t1 - v0 * v0 * t0);
                (var / tenor).sqrt()
            }
        }
    }

    /// Check the surface is well-formed (finite non-negative vols; for
    /// `Grid`: non-empty strictly-increasing axes, positive tenors, and
    /// matching grid dimensions). [`vol`](Self::vol) is total on any
    /// surface that passes.
    pub fn validate(&self) -> core::Result<()> {
        let err = |msg: String| Err(core::Error::MarketData(format!("vol surface: {msg}")));
        let finite_nonneg = |v: f64| v.is_finite() && v >= 0.0;
        let strictly_increasing =
            |xs: &[f64]| xs.windows(2).all(|w| w[0] < w[1]) && xs.iter().all(|x| x.is_finite());

        match self {
            VolSurface::Flat { vol } => {
                if !finite_nonneg(*vol) {
                    return err(format!("Flat vol {vol} must be non-negative and finite"));
                }
            }
            VolSurface::Grid {
                tenors,
                moneyness,
                vols,
            } => {
                if tenors.is_empty() || moneyness.is_empty() {
                    return err("Grid axes must be non-empty".to_string());
                }
                if !strictly_increasing(tenors) || tenors[0] <= 0.0 {
                    return err(format!(
                        "Grid tenors {tenors:?} must be finite, positive, strictly increasing"
                    ));
                }
                if !strictly_increasing(moneyness) {
                    return err(format!(
                        "Grid moneyness {moneyness:?} must be finite, strictly increasing"
                    ));
                }
                if vols.len() != tenors.len() {
                    return err(format!(
                        "Grid has {} vol rows for {} tenors",
                        vols.len(),
                        tenors.len()
                    ));
                }
                for (i, row) in vols.iter().enumerate() {
                    if row.len() != moneyness.len() {
                        return err(format!(
                            "Grid vol row {} has {} entries for {} moneyness points",
                            i,
                            row.len(),
                            moneyness.len()
                        ));
                    }
                    if let Some(v) = row.iter().find(|v| !finite_nonneg(**v)) {
                        return err(format!(
                            "Grid vol {v} (row {i}) must be non-negative and finite"
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_ignores_tenor_and_moneyness() {
        let s = VolSurface::Flat { vol: 0.30 };
        assert_eq!(s.vol(0.5, 0.0), 0.30);
        assert_eq!(s.vol(2.0, -0.15), 0.30);
    }

    #[test]
    fn serde_roundtrip() {
        let s = VolSurface::Flat { vol: 0.30 };
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, r#"{"type":"Flat","vol":0.3}"#);
        let s2: VolSurface = serde_json::from_str(&json).unwrap();
        assert_eq!(s2.vol(1.0, 0.0), 0.30);
    }

    /// Single-tenor smile: the qlox-ice shape (one option expiry per
    /// underlying future).
    fn smile() -> VolSurface {
        VolSurface::Grid {
            tenors: vec![0.033],
            moneyness: vec![-0.08, -0.01, 0.04, 0.10],
            vols: vec![vec![0.578, 0.584, 0.626, 0.675]],
        }
    }

    #[test]
    fn grid_hits_nodes_exactly() {
        let s = smile();
        assert_eq!(s.vol(0.033, -0.08), 0.578);
        assert_eq!(s.vol(0.033, 0.10), 0.675);
    }

    #[test]
    fn grid_interpolates_moneyness_linearly_in_vol() {
        let s = smile();
        // midway between -0.01 (0.584) and 0.04 (0.626)
        let v = s.vol(0.033, 0.015);
        assert!((v - 0.605).abs() < 1e-12, "got {v}");
    }

    #[test]
    fn grid_extrapolates_moneyness_flat() {
        let s = smile();
        assert_eq!(s.vol(0.033, -1.0), 0.578);
        assert_eq!(s.vol(0.033, 1.0), 0.675);
    }

    #[test]
    fn grid_single_tenor_is_flat_in_tenor() {
        let s = smile();
        // Degenerate tenor axis: any tenor reads the one row.
        for t in [0.001, 0.033, 0.5, 3.0] {
            assert_eq!(s.vol(t, 0.04), 0.626);
        }
    }

    fn term_grid() -> VolSurface {
        VolSurface::Grid {
            tenors: vec![0.25, 1.0],
            moneyness: vec![-0.1, 0.1],
            vols: vec![vec![0.20, 0.22], vec![0.40, 0.44]],
        }
    }

    #[test]
    fn grid_interpolates_tenor_in_total_variance() {
        let s = term_grid();
        // At moneyness -0.1: v(0.25)=0.20, v(1.0)=0.40.
        // var(0.25)=0.01, var(1.0)=0.16; at t=0.625, w=0.5:
        // var = 0.01 + 0.5*(0.16-0.01) = 0.085 → σ = sqrt(0.085/0.625)
        let expected = (0.085_f64 / 0.625).sqrt();
        let v = s.vol(0.625, -0.1);
        assert!((v - expected).abs() < 1e-12, "got {v}, want {expected}");
    }

    #[test]
    fn grid_extrapolates_tenor_flat() {
        let s = term_grid();
        assert_eq!(s.vol(0.1, -0.1), 0.20); // short end
        assert_eq!(s.vol(5.0, -0.1), 0.40); // long end
    }

    #[test]
    fn grid_bilinear_combines_both_axes() {
        let s = term_grid();
        // moneyness 0.0 rows: t=0.25 → 0.21, t=1.0 → 0.42; then total
        // variance between them at t=0.625.
        let (v0, v1) = (0.21_f64, 0.42_f64);
        let var = v0 * v0 * 0.25 + 0.5 * (v1 * v1 * 1.0 - v0 * v0 * 0.25);
        let expected = (var / 0.625).sqrt();
        let v = s.vol(0.625, 0.0);
        assert!((v - expected).abs() < 1e-12, "got {v}, want {expected}");
    }

    #[test]
    fn grid_serde_roundtrip() {
        let s = smile();
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.starts_with(r#"{"type":"Grid""#), "got {json}");
        let s2: VolSurface = serde_json::from_str(&json).unwrap();
        assert_eq!(s2.vol(0.033, 0.10), s.vol(0.033, 0.10));
        s2.validate().unwrap();
    }

    #[test]
    fn validate_accepts_well_formed() {
        VolSurface::Flat { vol: 0.3 }.validate().unwrap();
        smile().validate().unwrap();
        term_grid().validate().unwrap();
    }

    #[test]
    fn validate_rejects_malformed() {
        let bad: &[VolSurface] = &[
            VolSurface::Flat { vol: -0.1 },
            VolSurface::Flat { vol: f64::NAN },
            VolSurface::Grid {
                tenors: vec![],
                moneyness: vec![0.0],
                vols: vec![],
            },
            VolSurface::Grid {
                tenors: vec![0.0, 1.0], // tenor must be > 0
                moneyness: vec![0.0],
                vols: vec![vec![0.3], vec![0.3]],
            },
            VolSurface::Grid {
                tenors: vec![1.0, 0.5], // not increasing
                moneyness: vec![0.0],
                vols: vec![vec![0.3], vec![0.3]],
            },
            VolSurface::Grid {
                tenors: vec![0.5],
                moneyness: vec![0.1, 0.1], // not strictly increasing
                vols: vec![vec![0.3, 0.3]],
            },
            VolSurface::Grid {
                tenors: vec![0.5],
                moneyness: vec![0.0],
                vols: vec![], // row count mismatch
            },
            VolSurface::Grid {
                tenors: vec![0.5],
                moneyness: vec![0.0, 0.1],
                vols: vec![vec![0.3]], // row length mismatch
            },
            VolSurface::Grid {
                tenors: vec![0.5],
                moneyness: vec![0.0],
                vols: vec![vec![-0.3]], // negative vol
            },
        ];
        for s in bad {
            assert!(s.validate().is_err(), "should reject {s:?}");
        }
    }
}

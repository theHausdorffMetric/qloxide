//! Fengler (2009) arbitrage-free smoothing as a convex QP.
//!
//! Natural cubic smoothing spline on *forward call prices* in strike,
//! eqs (15)-(18) of the paper as verified in
//! RND/methods/arbitrage-free-smoothing.md, restated in forward space
//! (discount factor 1, no dividends — the underlying is a future):
//!
//!   min −y'x + ½x'Bx   over x = (g', γ')',  B = diag(W, λR)
//!   s.t.  Q'g = Rγ                  (Reinsch spline consistency)
//!         γᵢ ≥ 0                    (convexity ⟹ q ≥ 0, globally)
//!         (g₂−g₁)/h₁ − (h₁/6)γ₂ ≥ −1
//!         (gₙ−gₙ₋₁)/hₙ₋₁ + (hₙ₋₁/6)γₙ₋₁ ≤ 0
//!         F − u₁ ≤ g₁ ≤ F,   gₙ ≥ 0
//!
//! The guarantee is *global*: a natural cubic spline has a piecewise-
//! linear second derivative, and a piecewise-linear function that is
//! non-negative at its knots is non-negative everywhere between them.
//! Calendar constraints (eq 19) are omitted — single-tenor data.

use clarabel::algebra::CscMatrix;
use clarabel::solver::{
    DefaultSettingsBuilder, DefaultSolver, IPSolver, SolverStatus, SupportedConeT,
};

use crate::{Error, Result};

#[derive(Clone, Debug)]
pub struct FenglerConfig {
    /// Roughness-penalty weight λ. Selection is manual and logged —
    /// two central banks independently abandoned cross-validation here
    /// (RND/implementations/central-banks.md).
    pub lambda: f64,
    /// Per-quote fit weights wᵢ; None = equal weights (the paper leaves
    /// them unspecified).
    pub weights: Option<Vec<f64>>,
}

impl Default for FenglerConfig {
    fn default() -> Self {
        FenglerConfig {
            // Moments are λ-insensitive over ~1e-5..1e1 on dense grids
            // (the active convexity constraints do the regularizing), but
            // pointwise density smoothness is not: tick-quantized quotes
            // produce a spike/comb artifact in γ below λ ≈ 1, and λ ≈ 10
            // already costs several ticks of repricing error.
            lambda: 1.0,
            weights: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct FenglerFit {
    /// Knots (the input strikes).
    pub u: Vec<f64>,
    /// Fitted forward call values at the knots.
    pub g: Vec<f64>,
    /// Second derivatives γ at all n knots (γ₁ = γₙ = 0 by the natural-
    /// spline definition). γ *is* the density at the knots.
    pub gamma: Vec<f64>,
    /// Positivity certificate: min over interior γ as returned by the
    /// solver (negative only up to solver tolerance).
    pub min_gamma: f64,
    /// Max |g - y| over the knots, price units.
    pub max_fit_err: f64,
    pub solver_status: String,
}

/// Fit the constrained spline to forward call quotes `y` at strikes `u`
/// (strictly increasing), with forward price `f`.
pub fn fit(u: &[f64], y: &[f64], f: f64, cfg: &FenglerConfig) -> Result<FenglerFit> {
    let n = u.len();
    if n < 4 {
        return Err(Error::Solver(format!(
            "need at least 4 strikes for a constrained natural cubic spline, got {n}"
        )));
    }
    if y.len() != n {
        return Err(Error::Solver(format!(
            "{} quotes for {} strikes",
            y.len(),
            n
        )));
    }
    if !u.windows(2).all(|w| w[0] < w[1]) {
        return Err(Error::Solver("strikes must be strictly increasing".into()));
    }
    let w = match &cfg.weights {
        Some(w) if w.len() != n => {
            return Err(Error::Solver(format!(
                "{} weights for {n} strikes",
                w.len()
            )));
        }
        Some(w) => w.clone(),
        None => vec![1.0; n],
    };

    let h: Vec<f64> = u.windows(2).map(|p| p[1] - p[0]).collect();
    let ni = n - 2; // interior knots, carrying free γ
    let nv = n + ni; // decision variables x = (g₁..gₙ, γ₂..γₙ₋₁)

    // R: (ni × ni) symmetric tridiagonal, R[a][a] = (h[a]+h[a+1])/3,
    // R[a][a+1] = h[a+1]/6 — interior index a ↔ knot a+1 (0-based).
    let r = |a: usize, b: usize| -> f64 {
        if a == b {
            (h[a] + h[a + 1]) / 3.0
        } else if b == a + 1 || a == b + 1 {
            h[a.max(b)] / 6.0
        } else {
            0.0
        }
    };

    // Objective: P = diag(W, λR) (upper triangle — Clarabel reads triu
    // of a symmetric P), q = (−w₁y₁, …, −wₙyₙ, 0, …, 0).
    let mut p_dense = vec![vec![0.0; nv]; nv];
    for i in 0..n {
        p_dense[i][i] = w[i];
    }
    for a in 0..ni {
        for b in a..ni.min(a + 2) {
            p_dense[n + a][n + b] = cfg.lambda * r(a, b);
        }
    }
    let q: Vec<f64> = (0..nv)
        .map(|i| if i < n { -w[i] * y[i] } else { 0.0 })
        .collect();

    // Constraint rows, stacked: equalities first (zero cone), then
    // inequalities as a·x ≤ b (nonnegative cone on b − a·x).
    let mut a_dense: Vec<Vec<f64>> = Vec::new();
    let mut b_vec: Vec<f64> = Vec::new();

    // Q'g − Rγ = 0: row per interior index a (knot j = a+1).
    for a in 0..ni {
        let mut row = vec![0.0; nv];
        row[a] = 1.0 / h[a];
        row[a + 1] = -1.0 / h[a] - 1.0 / h[a + 1];
        row[a + 2] = 1.0 / h[a + 1];
        for b in a.saturating_sub(1)..ni.min(a + 2) {
            row[n + b] = -r(a, b);
        }
        a_dense.push(row);
        b_vec.push(0.0);
    }
    let n_eq = a_dense.len();

    // −γₐ ≤ 0 (convexity).
    for a in 0..ni {
        let mut row = vec![0.0; nv];
        row[n + a] = -1.0;
        a_dense.push(row);
        b_vec.push(0.0);
    }
    // Left slope ≥ −1: (g₁−g₂)/h₁ + (h₁/6)γ₂ ≤ 1.
    {
        let mut row = vec![0.0; nv];
        row[0] = 1.0 / h[0];
        row[1] = -1.0 / h[0];
        row[n] = h[0] / 6.0;
        a_dense.push(row);
        b_vec.push(1.0);
    }
    // Right slope ≤ 0: (gₙ−gₙ₋₁)/hₙ₋₁ + (hₙ₋₁/6)γₙ₋₁ ≤ 0.
    {
        let mut row = vec![0.0; nv];
        row[n - 1] = 1.0 / h[n - 2];
        row[n - 2] = -1.0 / h[n - 2];
        row[n + ni - 1] = h[n - 2] / 6.0;
        a_dense.push(row);
        b_vec.push(0.0);
    }
    // Level bounds: F − u₁ ≤ g₁ ≤ F, gₙ ≥ 0.
    {
        let mut row = vec![0.0; nv];
        row[0] = 1.0;
        a_dense.push(row);
        b_vec.push(f);
    }
    {
        let mut row = vec![0.0; nv];
        row[0] = -1.0;
        a_dense.push(row);
        b_vec.push(u[0] - f);
    }
    {
        let mut row = vec![0.0; nv];
        row[n - 1] = -1.0;
        a_dense.push(row);
        b_vec.push(0.0);
    }
    let n_ineq = a_dense.len() - n_eq;

    let p_csc = dense_to_csc(&p_dense, true);
    let a_csc = dense_to_csc(&a_dense, false);
    let cones = [
        SupportedConeT::ZeroConeT(n_eq),
        SupportedConeT::NonnegativeConeT(n_ineq),
    ];
    // Tight tolerances matter here: wing knots have true γ ~ 1e-5, and at
    // the default gap tolerance 1e-8 the complementarity residual puts
    // spurious duals ~1e-8/γ on their γ ≥ 0 constraints, visibly bending
    // the fitted density in the wings (percent-level at ±2σ).
    let settings = DefaultSettingsBuilder::default()
        .verbose(false)
        .tol_gap_abs(1e-12)
        .tol_gap_rel(1e-12)
        .tol_feas(1e-12)
        .max_iter(200)
        .build()
        .map_err(|e| Error::Solver(format!("settings: {e}")))?;
    let mut solver = DefaultSolver::new(&p_csc, &q, &a_csc, &b_vec, &cones, settings)
        .map_err(|e| Error::Solver(format!("setup: {e:?}")))?;
    solver.solve();

    let status = solver.solution.status;
    if !matches!(status, SolverStatus::Solved | SolverStatus::AlmostSolved) {
        return Err(Error::Solver(format!("QP not solved: {status:?}")));
    }

    let x = &solver.solution.x;
    let g: Vec<f64> = x[..n].to_vec();
    let mut gamma = vec![0.0; n];
    gamma[1..(ni + 1)].copy_from_slice(&x[n..]);
    let min_gamma = x[n..].iter().copied().fold(f64::MAX, f64::min);
    let max_fit_err = g
        .iter()
        .zip(y)
        .map(|(gi, yi)| (gi - yi).abs())
        .fold(0.0f64, f64::max);

    Ok(FenglerFit {
        u: u.to_vec(),
        g,
        gamma,
        min_gamma,
        max_fit_err,
        solver_status: format!("{status:?}"),
    })
}

/// Dense → CSC. `triu` keeps only the upper triangle (Clarabel's P
/// convention for symmetric matrices).
fn dense_to_csc(a: &[Vec<f64>], triu: bool) -> CscMatrix<f64> {
    let m = a.len();
    let n = a[0].len();
    let mut colptr = Vec::with_capacity(n + 1);
    let mut rowval = Vec::new();
    let mut nzval = Vec::new();
    colptr.push(0);
    for j in 0..n {
        for (i, row) in a.iter().enumerate() {
            if triu && i > j {
                continue;
            }
            let v = row[j];
            if v != 0.0 {
                rowval.push(i);
                nzval.push(v);
            }
        }
        colptr.push(rowval.len());
    }
    CscMatrix::new(m, n, colptr, rowval, nzval)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Quotes from a smooth admissible curve (convex, decreasing, slope
    /// within [-1, 0]): the fit must reproduce them closely at tiny λ
    /// and certify γ ≥ 0.
    #[test]
    fn convex_quotes_fit_tightly() {
        let u: Vec<f64> = (0..9).map(|i| 90.0 + 2.5 * i as f64).collect();
        let y: Vec<f64> = u.iter().map(|k| 2000.0 / k).collect();
        let fit = fit(
            &u,
            &y,
            100.0,
            &FenglerConfig {
                lambda: 1e-8,
                weights: None,
            },
        )
        .unwrap();
        assert!(fit.min_gamma >= -1e-7, "min gamma {}", fit.min_gamma);
        assert!(fit.max_fit_err < 0.05, "max fit err {}", fit.max_fit_err);
    }

    #[test]
    fn butterfly_arbitrage_is_smoothed_away() {
        // A non-convex quote in the middle: the constrained fit cannot
        // reproduce it, but must stay feasible with γ ≥ 0.
        let u = vec![90.0, 92.5, 95.0, 97.5, 100.0, 102.5, 105.0];
        let mut y: Vec<f64> = u.iter().map(|k| 2000.0 / k).collect();
        y[3] += 0.5; // manufacture a convexity violation
        let fit = fit(&u, &y, 100.0, &FenglerConfig::default()).unwrap();
        assert!(fit.min_gamma >= -1e-7);
        // The violating quote is the one the fit moves most.
        let errs: Vec<f64> = fit.g.iter().zip(&y).map(|(g, y)| (g - y).abs()).collect();
        let max_idx = errs
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0;
        assert_eq!(max_idx, 3, "errs {errs:?}");
    }

    #[test]
    fn too_few_strikes_rejected() {
        assert!(
            fit(
                &[90.0, 95.0, 100.0],
                &[10.0, 5.0, 2.0],
                100.0,
                &FenglerConfig::default()
            )
            .is_err()
        );
    }
}

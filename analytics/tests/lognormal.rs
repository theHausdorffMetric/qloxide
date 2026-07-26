//! Ground truth: a flat-vol Black76 slice has an analytically known RND
//! (lognormal). The full pipeline must recover it.

use qloxide::instruments::PutOrCall;
use qloxide::math::norm_cdf;
use qloxide::pricing::black76::{Black76Params, black76_price};
use qloxide_analytics::density::extract;
use qloxide_analytics::fengler::{FenglerConfig, fit};

const F: f64 = 100.0;
const T: f64 = 0.25;
const SIGMA: f64 = 0.25;

fn forward_call(k: f64) -> f64 {
    black76_price(
        Black76Params {
            f: F,
            k,
            t: T,
            r: 0.0,
            sigma: SIGMA,
        },
        PutOrCall::Call,
    )
    .unwrap()
}

fn lognormal_pdf(s: f64) -> f64 {
    let sst = SIGMA * T.sqrt();
    let z = ((s / F).ln() + 0.5 * sst * sst) / sst;
    (-0.5 * z * z).exp() / (s * sst * (2.0 * std::f64::consts::PI).sqrt())
}

fn lognormal_cdf(s: f64) -> f64 {
    let sst = SIGMA * T.sqrt();
    norm_cdf(((s / F).ln() + 0.5 * sst * sst) / sst)
}

fn strikes() -> Vec<f64> {
    (60..=150).map(|k| k as f64).collect()
}

#[test]
fn recovers_the_lognormal_density() {
    let u = strikes();
    let y: Vec<f64> = u.iter().map(|&k| forward_call(k)).collect();
    let f = fit(
        &u,
        &y,
        F,
        &FenglerConfig {
            lambda: 1e-8,
            weights: None,
        },
    )
    .unwrap();
    let d = extract(&f);

    // Positivity certificate.
    assert!(f.min_gamma >= -1e-8, "min gamma {}", f.min_gamma);

    // Total mass is 1 by construction (interior + implied tail masses).
    let total = d.mass_left + d.mass_interior + d.mass_right;
    assert!((total - 1.0).abs() < 1e-6, "total mass {total}");

    // Knot densities match the analytic lognormal within 2% where the
    // density is meaningful (±2σ of the forward).
    let sst = SIGMA * T.sqrt();
    for (i, &k) in u.iter().enumerate() {
        let z = (k / F).ln().abs() / sst;
        if z > 2.0 {
            continue;
        }
        let truth = lognormal_pdf(k);
        let rel = (d.q[i] - truth).abs() / truth;
        assert!(
            rel < 0.02,
            "strike {k}: q {} vs {truth} (rel {rel})",
            d.q[i]
        );
    }

    // CDF via the digital identity matches the analytic CDF.
    for (i, &k) in u.iter().enumerate() {
        assert!(
            (d.cdf[i] - lognormal_cdf(k)).abs() < 2e-3,
            "strike {k}: cdf {} vs {}",
            d.cdf[i],
            lognormal_cdf(k)
        );
    }

    // Implied tail masses match the analytic tails.
    assert!((d.mass_left - lognormal_cdf(60.0)).abs() < 1e-3);
    assert!((d.mass_right - (1.0 - lognormal_cdf(150.0))).abs() < 1e-3);

    // Martingale mean (boundary atoms bias inward, but tails are tiny here).
    assert!((d.mean - F).abs() < 0.05, "mean {}", d.mean);

    // Median of the lognormal is F·exp(-σ²t/2).
    let med = d.quantile(0.5).unwrap();
    let truth = F * (-0.5 * sst * sst).exp();
    assert!((med - truth).abs() < 0.1, "median {med} vs {truth}");
}

#[test]
fn stitched_tails_recover_lognormal_moments() {
    let u = strikes();
    let y: Vec<f64> = u.iter().map(|&k| forward_call(k)).collect();
    let f = fit(
        &u,
        &y,
        F,
        &FenglerConfig {
            lambda: 1e-8,
            weights: None,
        },
    )
    .unwrap();
    let d = extract(&f);
    let slice = qloxide_analytics::slice::SmileSlice {
        underlying: "TEST".into(),
        valuation_date: "2026-01-01".into(),
        f: F,
        t: T,
        strikes: u.clone(),
        vols: vec![SIGMA; u.len()],
        calls: y,
    };
    let st = qloxide_analytics::tails::graft(&slice, &f, &d, 0.05, 0.02).unwrap();

    // Analytic lognormal moments for σ√t = 0.125.
    let v = SIGMA * SIGMA * T;
    let sd_truth = F * (v.exp() - 1.0).sqrt();
    let skew_truth = (v.exp() + 2.0) * (v.exp() - 1.0).sqrt();
    let kurt_truth = (4.0 * v).exp() + 2.0 * (3.0 * v).exp() + 3.0 * (2.0 * v).exp() - 3.0;

    assert!((st.mean - F).abs() < 0.02, "mean {}", st.mean);
    assert!(
        (st.sd - sd_truth).abs() / sd_truth < 0.01,
        "sd {} vs {sd_truth}",
        st.sd
    );
    assert!(
        (st.skew - skew_truth).abs() < 0.06,
        "skew {} vs {skew_truth}",
        st.skew
    );
    assert!(
        (st.kurt - kurt_truth).abs() < 0.15,
        "kurt {} vs {kurt_truth}",
        st.kurt
    );
    assert!(st.mass_below_zero < 1e-9);

    // Stitched quantiles now reach into the tails.
    let p01 = st.quantile(0.01).unwrap();
    let truth = F * (-0.5 * v + 0.125 * norm_inv_approx(0.01)).exp();
    assert!((p01 - truth).abs() / truth < 0.01, "p01 {p01} vs {truth}");
}

/// Inverse normal CDF by bisecting qloxide's norm_cdf — plenty accurate
/// for the quantile tolerance above.
fn norm_inv_approx(p: f64) -> f64 {
    let (mut lo, mut hi) = (-8.0, 8.0);
    for _ in 0..80 {
        let mid = 0.5 * (lo + hi);
        if norm_cdf(mid) < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

#[test]
fn survives_half_tick_noise() {
    let u = strikes();
    // Deterministic ±0.005 perturbation (no RNG: reproducible).
    let y: Vec<f64> = u
        .iter()
        .enumerate()
        .map(|(i, &k)| forward_call(k) + 0.01 * ((i * 7919 % 1000) as f64 / 1000.0 - 0.5))
        .collect();
    let f = fit(
        &u,
        &y,
        F,
        &FenglerConfig {
            lambda: 1e-4,
            weights: None,
        },
    )
    .unwrap();
    let d = extract(&f);

    // The certificate must hold regardless of noise…
    assert!(f.min_gamma >= -1e-8, "min gamma {}", f.min_gamma);
    let total = d.mass_left + d.mass_interior + d.mass_right;
    assert!((total - 1.0).abs() < 1e-6);
    // …and the recovered distribution stays close to the truth.
    assert!((d.mean - F).abs() < 0.5, "mean {}", d.mean);
    assert!(
        (d.sd - F * (SIGMA * SIGMA * T).exp_m1().sqrt()).abs() < 1.0,
        "sd {}",
        d.sd
    );
}

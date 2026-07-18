//! Implied volatility: invert Black76 for the vol that reproduces an
//! observed option premium.
//!
//! Used to build market-consistent vol surfaces from settlement premiums
//! (a listed option's mark *is* its settlement premium, so the surface
//! must return the vol that reprices it under this library's own T/r
//! conventions — consuming an exchange's published vol column imports
//! that exchange's undocumented annualization instead).
//!
//! Method: Brent bracketing on the (monotone-in-σ) Black76 price, seeded
//! with the Corrado-Miller closed-form approximation, solving on the
//! out-of-the-money leg (via put-call parity) where the objective is
//! well-conditioned. Newton-Raphson is deliberately not used: near zero
//! vega it diverges to negative vols.

use std::f64::consts::PI;

use crate::core;
use crate::instruments::PutOrCall;
use crate::math::brent_root;
use crate::pricing::black76::{Black76Params, black76_price};

/// Convergence tolerance on sigma.
const SIGMA_TOL: f64 = 1e-12;
/// Bracket ceiling: 3200% vol — far beyond any meaningful quote (deep
/// ITM settlement noise on ICE reaches ~500%).
const SIGMA_MAX: f64 = 32.0;
const MAX_ITER: usize = 200;

/// Implied Black76 (lognormal) volatility from an option premium.
///
/// `f`, `k` — futures price and strike; `t` — time to expiry in years
/// (must be positive: an expired option carries no vol information);
/// `r` — continuously compounded rate to expiry; `premium` — the
/// observed (discounted) option price.
///
/// Errors if the premium violates the no-arbitrage bounds
/// `df·intrinsic ≤ premium < df·F` (call) / `< df·K` (put), all inputs
/// considered. A premium exactly at discounted intrinsic returns 0.
pub fn black76_implied_vol(
    f: f64,
    k: f64,
    t: f64,
    r: f64,
    premium: f64,
    put_or_call: PutOrCall,
) -> core::Result<f64> {
    let err = |msg: String| Err(core::Error::Model(format!("implied vol: {msg}")));
    if !f.is_finite() || f <= 0.0 {
        return err(format!("futures price {f} must be positive and finite"));
    }
    if !k.is_finite() || k <= 0.0 {
        return err(format!("strike {k} must be positive and finite"));
    }
    if !t.is_finite() || t <= 0.0 {
        return err(format!(
            "time to expiry {t} must be positive (expired options carry no vol)"
        ));
    }
    if !r.is_finite() {
        return err(format!("rate {r} must be finite"));
    }
    if !premium.is_finite() || premium < 0.0 {
        return err(format!("premium {premium} must be non-negative and finite"));
    }

    let df = (-r * t).exp();

    // Flip to the out-of-the-money leg via put-call parity
    // (C − P = df·(F − K)): same vol, better-conditioned objective.
    // After the flip the working option's intrinsic is zero, so the
    // target is pure time value.
    let (side, target) = match put_or_call {
        PutOrCall::Call if k < f => (PutOrCall::Put, premium - df * (f - k)),
        PutOrCall::Put if k > f => (PutOrCall::Call, premium - df * (k - f)),
        side => (side, premium),
    };
    if target < 0.0 {
        return err(format!(
            "premium {premium} is below discounted intrinsic value"
        ));
    }
    if target == 0.0 {
        return Ok(0.0);
    }
    let upper = match side {
        PutOrCall::Call => df * f,
        PutOrCall::Put => df * k,
    };
    if target >= upper {
        return err(format!(
            "premium {premium} is at or above the no-arbitrage bound"
        ));
    }

    // g(σ) = model − target is monotone increasing (vega > 0), g(0) < 0.
    let g = |sigma: f64| {
        black76_price(Black76Params { f, k, t, r, sigma }, side)
            .map(|p| p - target)
            .unwrap_or(f64::NAN)
    };

    // Bracket the root: seed the ceiling with Corrado-Miller (an
    // approximation only — correctness comes from Brent), then double
    // until g turns positive.
    let mut hi = match corrado_miller(f, k, t, df, side, target) {
        Some(cm) => (2.0 * cm).clamp(0.5, SIGMA_MAX),
        None => 1.0,
    };
    while g(hi) < 0.0 {
        if hi >= SIGMA_MAX {
            return err(format!(
                "no vol below {SIGMA_MAX} reprices premium {premium}"
            ));
        }
        hi = (hi * 2.0).min(SIGMA_MAX);
    }

    match brent_root(g, 0.0, hi, SIGMA_TOL, MAX_ITER) {
        Some(sigma) => Ok(sigma.max(0.0)),
        None => err(format!(
            "did not converge for premium {premium} (F={f}, K={k}, T={t})"
        )),
    }
}

/// Corrado-Miller (1996) closed-form implied-vol approximation, b=0.
///
/// Works on a *call* premium; the put leg is flipped in via parity.
/// Returns `None` where the inner square root goes negative (far from
/// the money) — callers fall back to a fixed bracket.
fn corrado_miller(f: f64, k: f64, t: f64, df: f64, side: PutOrCall, target: f64) -> Option<f64> {
    let s = f * df;
    let x = k * df;
    let call = match side {
        PutOrCall::Call => target,
        PutOrCall::Put => target + df * (f - k),
    };
    let a = call - (s - x) / 2.0;
    let inner = a * a - (s - x) * (s - x) / PI;
    if inner < 0.0 {
        return None;
    }
    let cm = (2.0 * PI / t).sqrt() / (s + x) * (a + inner.sqrt());
    (cm.is_finite() && cm > 0.0).then_some(cm)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Price → imply must recover sigma across moneyness, tenor, rate,
    /// and side — including the ITM legs that exercise the parity flip.
    #[test]
    fn roundtrip_recovers_sigma() {
        for side in [PutOrCall::Call, PutOrCall::Put] {
            for (f, k, t, r, sigma) in [
                (100.0, 100.0, 1.0, 0.05, 0.30),
                (84.23, 93.0, 12.0 / 360.0, 0.04, 0.675), // ICE wing shape
                (84.23, 78.0, 12.0 / 360.0, 0.04, 0.578),
                (100.0, 50.0, 0.5, 0.02, 0.40), // deep ITM call / OTM put
                (100.0, 200.0, 0.5, 0.02, 0.40),
                (72.45, 75.0, 2.0, 0.0, 0.15),
            ] {
                let params = Black76Params { f, k, t, r, sigma };
                let premium = black76_price(params, side).unwrap();
                let implied = black76_implied_vol(f, k, t, r, premium, side).unwrap();
                assert!(
                    (implied - sigma).abs() < 1e-8,
                    "{side:?} F={f} K={k}: implied {implied} vs {sigma}"
                );
            }
        }
    }

    #[test]
    fn premium_at_discounted_intrinsic_is_zero_vol() {
        // ITM call at exactly discounted intrinsic.
        let df = (-0.05_f64 * 1.0).exp();
        let v = black76_implied_vol(110.0, 100.0, 1.0, 0.05, df * 10.0, PutOrCall::Call).unwrap();
        assert_eq!(v, 0.0);
        // OTM put with zero premium.
        let v = black76_implied_vol(110.0, 100.0, 1.0, 0.05, 0.0, PutOrCall::Put).unwrap();
        assert_eq!(v, 0.0);
    }

    #[test]
    fn below_intrinsic_rejected() {
        // ITM call quoted below discounted intrinsic (stale/arbitrage).
        let e = black76_implied_vol(110.0, 100.0, 1.0, 0.05, 5.0, PutOrCall::Call).unwrap_err();
        assert!(e.to_string().contains("below discounted intrinsic"), "{e}");
    }

    #[test]
    fn above_upper_bound_rejected() {
        // A call can never be worth more than the discounted forward.
        let e = black76_implied_vol(100.0, 100.0, 1.0, 0.0, 100.0, PutOrCall::Call).unwrap_err();
        assert!(e.to_string().contains("no-arbitrage bound"), "{e}");
    }

    #[test]
    fn invalid_inputs_rejected() {
        let call = PutOrCall::Call;
        assert!(black76_implied_vol(-1.0, 100.0, 1.0, 0.0, 1.0, call).is_err());
        assert!(black76_implied_vol(100.0, 0.0, 1.0, 0.0, 1.0, call).is_err());
        assert!(black76_implied_vol(100.0, 100.0, 0.0, 0.0, 1.0, call).is_err()); // expired
        assert!(black76_implied_vol(100.0, 100.0, -1.0, 0.0, 1.0, call).is_err());
        assert!(black76_implied_vol(100.0, 100.0, 1.0, f64::NAN, 1.0, call).is_err());
        assert!(black76_implied_vol(100.0, 100.0, 1.0, 0.0, -1.0, call).is_err());
    }

    /// C and P at the same strike must imply the same vol (parity) —
    /// the property ICE settlement chains exhibit.
    #[test]
    fn call_and_put_imply_identical_vol() {
        let (f, k, t, r, sigma) = (84.23, 88.0, 12.0 / 360.0, 0.04, 0.626);
        let params = Black76Params { f, k, t, r, sigma };
        let c = black76_price(params, PutOrCall::Call).unwrap();
        let p = black76_price(params, PutOrCall::Put).unwrap();
        let vc = black76_implied_vol(f, k, t, r, c, PutOrCall::Call).unwrap();
        let vp = black76_implied_vol(f, k, t, r, p, PutOrCall::Put).unwrap();
        assert!((vc - vp).abs() < 1e-9, "call {vc} vs put {vp}");
    }

    /// Tiny time value on a near-expiry wing — the ill-conditioned
    /// region the Brent bracket must still handle.
    #[test]
    fn tiny_time_value_converges() {
        let (f, k, t, r) = (84.23, 120.0, 2.0 / 360.0, 0.04);
        let sigma = 0.80;
        let premium = black76_price(Black76Params { f, k, t, r, sigma }, PutOrCall::Call).unwrap();
        assert!(premium < 1e-6, "premise: wing is nearly worthless");
        let implied = black76_implied_vol(f, k, t, r, premium, PutOrCall::Call).unwrap();
        assert!((implied - sigma).abs() < 1e-4, "implied {implied}");
    }
}

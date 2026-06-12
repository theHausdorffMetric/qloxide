//! Black (1976) model for European options on futures/forwards.
//!
//! The b=0 specialization of generalized Black-Scholes-Merton: the
//! underlying is a futures price with no cost of carry. Pure math on f64
//! inputs — no instrument or market data types. Formulas follow Haug,
//! *Option Pricing Formulas* (2007).

use crate::core;
use crate::instruments::PutOrCall;
use crate::math::{norm_cdf, norm_pdf};

/// Threshold below which sigma²·T is treated as zero (expired or
/// vol-free): the option collapses to discounted intrinsic value.
const DEGENERATE: f64 = 1e-10;

/// Inputs to the Black76 formula.
#[derive(Clone, Copy, Debug)]
pub struct Black76Params {
    /// Futures/forward price.
    pub f: f64,
    /// Strike.
    pub k: f64,
    /// Time to expiry in years.
    pub t: f64,
    /// Continuously compounded risk-free rate to expiry.
    pub r: f64,
    /// Lognormal volatility (e.g. 0.30 for 30%).
    pub sigma: f64,
}

/// Option value and sensitivities.
///
/// Conventions: `delta` and `gamma` are with respect to the futures price;
/// `vega` is per unit of vol (divide by 100 for per vol point); `theta` is
/// per year of calendar time passing (negative for long options, typically).
#[derive(Clone, Copy, Debug)]
pub struct Greeks {
    pub price: f64,
    pub delta: f64,
    pub gamma: f64,
    pub vega: f64,
    pub theta: f64,
}

impl Black76Params {
    fn validate(&self) -> core::Result<()> {
        // is_finite() also rejects NaN, so NaN inputs error rather than
        // propagating silently through the formulas.
        if !self.f.is_finite() || self.f <= 0.0 {
            return Err(core::Error::Model(format!(
                "futures price {} must be positive and finite",
                self.f
            )));
        }
        if !self.k.is_finite() || self.k <= 0.0 {
            return Err(core::Error::Model(format!(
                "strike {} must be positive and finite",
                self.k
            )));
        }
        if !self.t.is_finite() || self.t < 0.0 {
            return Err(core::Error::Model(format!(
                "time to expiry {} must be non-negative and finite",
                self.t
            )));
        }
        if !self.sigma.is_finite() || self.sigma < 0.0 {
            return Err(core::Error::Model(format!(
                "volatility {} must be non-negative and finite",
                self.sigma
            )));
        }
        if !self.r.is_finite() {
            return Err(core::Error::Model(format!("rate {} must be finite", self.r)));
        }
        Ok(())
    }

    fn d1(&self) -> f64 {
        ((self.f / self.k).ln() + 0.5 * self.sigma * self.sigma * self.t)
            / (self.sigma * self.t.sqrt())
    }

    /// Discounted intrinsic value (the degenerate sigma²·T → 0 limit).
    fn discounted_intrinsic(&self, put_or_call: PutOrCall) -> f64 {
        let df = (-self.r * self.t).exp();
        match put_or_call {
            PutOrCall::Call => df * (self.f - self.k).max(0.0),
            PutOrCall::Put => df * (self.k - self.f).max(0.0),
        }
    }

    fn is_degenerate(&self) -> bool {
        self.sigma * self.sigma * self.t < DEGENERATE
    }
}

/// Black76 option premium.
pub fn black76_price(params: Black76Params, put_or_call: PutOrCall) -> core::Result<f64> {
    params.validate()?;
    if params.is_degenerate() {
        return Ok(params.discounted_intrinsic(put_or_call));
    }

    let Black76Params { f, k, t, r, sigma } = params;
    let d1 = params.d1();
    let d2 = d1 - sigma * t.sqrt();
    let df = (-r * t).exp();

    Ok(match put_or_call {
        PutOrCall::Call => df * (f * norm_cdf(d1) - k * norm_cdf(d2)),
        PutOrCall::Put => df * (k * norm_cdf(-d2) - f * norm_cdf(-d1)),
    })
}

/// Black76 premium and Greeks.
pub fn black76_greeks(params: Black76Params, put_or_call: PutOrCall) -> core::Result<Greeks> {
    params.validate()?;
    if params.is_degenerate() {
        // At the degenerate limit the option is (discounted) intrinsic;
        // delta is the discounted indicator, gamma/vega/theta limits are 0
        // away from the strike (and undefined at it — we report 0).
        let df = (-params.r * params.t).exp();
        let delta = match put_or_call {
            PutOrCall::Call if params.f > params.k => df,
            PutOrCall::Put if params.f < params.k => -df,
            _ => 0.0,
        };
        return Ok(Greeks {
            price: params.discounted_intrinsic(put_or_call),
            delta,
            gamma: 0.0,
            vega: 0.0,
            theta: 0.0,
        });
    }

    let Black76Params { f, k, t, r, sigma } = params;
    let sqrt_t = t.sqrt();
    let d1 = params.d1();
    let d2 = d1 - sigma * sqrt_t;
    let df = (-r * t).exp();

    let price = match put_or_call {
        PutOrCall::Call => df * (f * norm_cdf(d1) - k * norm_cdf(d2)),
        PutOrCall::Put => df * (k * norm_cdf(-d2) - f * norm_cdf(-d1)),
    };

    let delta = match put_or_call {
        PutOrCall::Call => df * norm_cdf(d1),
        PutOrCall::Put => df * (norm_cdf(d1) - 1.0),
    };

    let gamma = df * norm_pdf(d1) / (f * sigma * sqrt_t);
    let vega = f * df * norm_pdf(d1) * sqrt_t;

    // Haug (2007) with b=0; θ = −dV/dT. Note the r·F·df·N(±d1) term —
    // the original plan document omitted it, which disagrees both with
    // the reference implementation and with finite differences.
    let theta1 = -(f * df * norm_pdf(d1) * sigma) / (2.0 * sqrt_t);
    let theta = match put_or_call {
        PutOrCall::Call => theta1 + r * f * df * norm_cdf(d1) - r * k * df * norm_cdf(d2),
        PutOrCall::Put => theta1 - r * f * df * norm_cdf(-d1) + r * k * df * norm_cdf(-d2),
    };

    Ok(Greeks {
        price,
        delta,
        gamma,
        vega,
        theta,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(f: f64, k: f64, t: f64, r: f64, sigma: f64) -> Black76Params {
        Black76Params { f, k, t, r, sigma }
    }

    // Reference value from docs/opt_src/gbsm.rs (Haug, b=0)
    #[test]
    fn call_reference_value() {
        let p = black76_price(params(100.0, 100.0, 1.0, 0.01, 0.3), PutOrCall::Call).unwrap();
        assert!(
            (p - 11.80489728393353).abs() < 1e-12,
            "got {p}"
        );
    }

    #[test]
    fn delta_reference_values() {
        let g = black76_greeks(params(105.0, 100.0, 0.5, 0.1, 0.36), PutOrCall::Call).unwrap();
        assert!((g.delta - 0.5946).abs() < 1e-4, "call delta {}", g.delta);
        let g = black76_greeks(params(105.0, 100.0, 0.5, 0.1, 0.36), PutOrCall::Put).unwrap();
        assert!((g.delta + 0.3566).abs() < 1e-4, "put delta {}", g.delta);
    }

    #[test]
    fn put_call_parity() {
        // C - P = df * (F - K)
        for (f, k, t, r, sigma) in [
            (100.0, 100.0, 1.0, 0.05, 0.3),
            (72.45, 75.0, 0.8, 0.043, 0.3),
            (55.0, 60.0, 0.75, 0.105, 0.2),
        ] {
            let c = black76_price(params(f, k, t, r, sigma), PutOrCall::Call).unwrap();
            let p = black76_price(params(f, k, t, r, sigma), PutOrCall::Put).unwrap();
            let expected = (-r * t).exp() * (f - k);
            assert!(
                (c - p - expected).abs() < 1e-12,
                "parity violated: C-P={}, df(F-K)={}",
                c - p,
                expected
            );
        }
    }

    #[test]
    fn zero_vol_gives_discounted_intrinsic() {
        let p = black76_price(params(110.0, 100.0, 1.0, 0.05, 0.0), PutOrCall::Call).unwrap();
        let expected = (-0.05_f64).exp() * 10.0;
        assert!((p - expected).abs() < 1e-12);
        let p = black76_price(params(110.0, 100.0, 1.0, 0.05, 0.0), PutOrCall::Put).unwrap();
        assert_eq!(p, 0.0);
    }

    #[test]
    fn zero_time_gives_intrinsic() {
        let p = black76_price(params(90.0, 100.0, 0.0, 0.05, 0.3), PutOrCall::Put).unwrap();
        assert!((p - 10.0).abs() < 1e-12);
    }

    #[test]
    fn vega_and_gamma_non_negative() {
        for f in [50.0, 100.0, 150.0] {
            let g = black76_greeks(params(f, 100.0, 0.5, 0.05, 0.25), PutOrCall::Call).unwrap();
            assert!(g.vega >= 0.0);
            assert!(g.gamma >= 0.0);
        }
    }

    #[test]
    fn invalid_inputs_rejected() {
        assert!(black76_price(params(-1.0, 100.0, 1.0, 0.0, 0.3), PutOrCall::Call).is_err());
        assert!(black76_price(params(100.0, 0.0, 1.0, 0.0, 0.3), PutOrCall::Call).is_err());
        assert!(black76_price(params(100.0, 100.0, -1.0, 0.0, 0.3), PutOrCall::Call).is_err());
        assert!(black76_price(params(100.0, 100.0, 1.0, 0.0, -0.3), PutOrCall::Call).is_err());
        assert!(black76_price(params(f64::NAN, 100.0, 1.0, 0.0, 0.3), PutOrCall::Call).is_err());
    }

    /// Greeks must agree with central finite differences of the price.
    /// This is the test that catches wrong formulas (it found the missing
    /// r·F·df·N(d1) theta term in the plan document).
    #[test]
    fn greeks_match_finite_differences() {
        let base = params(72.45, 75.0, 0.8, 0.043, 0.3);
        for pc in [PutOrCall::Call, PutOrCall::Put] {
            let g = black76_greeks(base, pc).unwrap();
            let h = 1e-5;

            let up = black76_price(params(base.f + h, base.k, base.t, base.r, base.sigma), pc).unwrap();
            let dn = black76_price(params(base.f - h, base.k, base.t, base.r, base.sigma), pc).unwrap();
            let fd_delta = (up - dn) / (2.0 * h);
            assert!((g.delta - fd_delta).abs() < 1e-7, "{pc:?} delta {} vs fd {}", g.delta, fd_delta);

            // Gamma needs a larger bump: with h=1e-5 the second difference
            // divides f64 cancellation noise by h²=1e-10 and drowns.
            let hg = 1e-3;
            let gup = black76_price(params(base.f + hg, base.k, base.t, base.r, base.sigma), pc).unwrap();
            let gdn = black76_price(params(base.f - hg, base.k, base.t, base.r, base.sigma), pc).unwrap();
            let mid = black76_price(base, pc).unwrap();
            let fd_gamma = (gup - 2.0 * mid + gdn) / (hg * hg);
            assert!((g.gamma - fd_gamma).abs() < 1e-6, "{pc:?} gamma {} vs fd {}", g.gamma, fd_gamma);

            let vu = black76_price(params(base.f, base.k, base.t, base.r, base.sigma + h), pc).unwrap();
            let vd = black76_price(params(base.f, base.k, base.t, base.r, base.sigma - h), pc).unwrap();
            let fd_vega = (vu - vd) / (2.0 * h);
            assert!((g.vega - fd_vega).abs() < 1e-6, "{pc:?} vega {} vs fd {}", g.vega, fd_vega);

            // theta = -dV/dT
            let tu = black76_price(params(base.f, base.k, base.t + h, base.r, base.sigma), pc).unwrap();
            let td = black76_price(params(base.f, base.k, base.t - h, base.r, base.sigma), pc).unwrap();
            let fd_theta = -(tu - td) / (2.0 * h);
            assert!((g.theta - fd_theta).abs() < 1e-6, "{pc:?} theta {} vs fd {}", g.theta, fd_theta);
        }
    }
}

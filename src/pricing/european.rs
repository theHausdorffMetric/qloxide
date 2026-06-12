//! European option pricing: bridges `EuropeanOption`, `PricingContext`,
//! and the Black76 math.

use crate::core;
use crate::instruments::option::EuropeanOption;
use crate::instruments::{ExerciseStyle, PutOrCall};
use crate::pricing::PricingContext;
use crate::pricing::black76::{Black76Params, Greeks, black76_greeks, black76_price};
use crate::pricing::decimal_to_f64;

/// Assemble Black76 inputs for an option from the pricing context.
///
/// Returns `None` if the option has expired — the caller handles expiry
/// (intrinsic against the underlying's settlement/market price).
fn black76_inputs(
    option: &EuropeanOption,
    ctx: &dyn PricingContext,
) -> core::Result<Option<Black76Params>> {
    if option.exercise_style != ExerciseStyle::European {
        return Err(core::Error::Pricer(format!(
            "option '{}': only European exercise is supported (got {:?})",
            option.id, option.exercise_style,
        )));
    }

    let t = option
        .currency
        .day_count
        .year_fraction(ctx.valuation_date(), option.expiry);
    if t <= 0.0 {
        return Ok(None);
    }

    let f = ctx.market_price(&option.underlying)?;
    let k = decimal_to_f64(option.strike)?;
    let r = ctx.discount_curve(&option.currency.id)?.zero_rate(option.expiry);
    let moneyness = (k / f).ln();
    let sigma = ctx.vol(&option.underlying, t, moneyness)?;

    Ok(Some(Black76Params { f, k, t, r, sigma }))
}

/// Underlying price for an expired option: final settlement price if
/// available, otherwise the market price.
fn expired_underlying_price(
    option: &EuropeanOption,
    ctx: &dyn PricingContext,
) -> core::Result<f64> {
    ctx.settlement_price(&option.underlying)
        .or_else(|_| ctx.market_price(&option.underlying))
}

/// Price a European option with Black76.
///
/// The model is implied by the underlying's vol surface (Flat = lognormal
/// = Black76). Expired options return undiscounted intrinsic value against
/// the underlying's settlement price.
pub fn price_european(option: &EuropeanOption, ctx: &dyn PricingContext) -> core::Result<f64> {
    match black76_inputs(option, ctx)? {
        Some(params) => black76_price(params, option.put_or_call),
        None => {
            let f = expired_underlying_price(option, ctx)?;
            let k = decimal_to_f64(option.strike)?;
            Ok(match option.put_or_call {
                PutOrCall::Call => (f - k).max(0.0),
                PutOrCall::Put => (k - f).max(0.0),
            })
        }
    }
}

/// Greeks for a European option with Black76.
///
/// Errors on expired options — Greeks of an expired contract are not
/// meaningful; check `maturity()` against the valuation date first.
pub fn greeks_european(option: &EuropeanOption, ctx: &dyn PricingContext) -> core::Result<Greeks> {
    match black76_inputs(option, ctx)? {
        Some(params) => black76_greeks(params, option.put_or_call),
        None => Err(core::Error::Pricer(format!(
            "option '{}' is expired; Greeks are not defined",
            option.id,
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curves::DiscountCurve;
    use crate::dates::Date;
    use crate::dates::daycount::DayCount;
    use crate::dates::rules::DateRule;
    use crate::instruments::{OptionSettlement, Settlement};
    use crate::market_data::{MarketData, VolSurface};
    use crate::pricing;
    use crate::reference_data::Currency;
    use rust_decimal::Decimal;
    use std::sync::Arc;

    fn usd() -> Arc<Currency> {
        Arc::new(Currency::new("USD", DateRule::Null, DayCount::Act365Fixed))
    }

    fn test_option(strike: u32, put_or_call: PutOrCall, expiry: Date) -> EuropeanOption {
        EuropeanOption::new(
            "OPT",
            "UNDERLYING",
            "CP",
            usd(),
            Settlement::new("ICE", "SETTLE", "19:30", "Europe/London", DateRule::Null),
            expiry,
            Decimal::from(strike),
            put_or_call,
            OptionSettlement::Cash,
        )
    }

    /// Market data with: F=100, flat 1% curve, flat 30% vol,
    /// valuation 2026-01-01.
    fn test_md() -> MarketData {
        let valuation = Date::new(2026, 1, 1);
        let mut md = MarketData::new(valuation, valuation.as_of_midnight());
        md.add_market_price("UNDERLYING", 100.0);
        md.add_discount_curve(
            "USD",
            DiscountCurve::flat(valuation, DayCount::Act365Fixed, 0.01),
        );
        md.add_vol_surface("UNDERLYING", VolSurface::Flat { vol: 0.30 });
        md
    }

    #[test]
    fn full_chain_matches_reference_value() {
        // Expiry exactly one Act/365 year out: T = 1.0, F = K = 100,
        // r = 1%, sigma = 30% — the reference Black76 call value.
        let option = test_option(100, PutOrCall::Call, Date::new(2027, 1, 1));
        let md = test_md();
        // Through the dispatch chain, not just the module function
        let px = pricing::price(&option, &md).unwrap();
        assert!((px - 11.80489728393353).abs() < 1e-10, "got {px}");
    }

    #[test]
    fn put_call_parity_through_chain() {
        let md = test_md();
        let expiry = Date::new(2027, 1, 1);
        let c = price_european(&test_option(105, PutOrCall::Call, expiry), &md).unwrap();
        let p = price_european(&test_option(105, PutOrCall::Put, expiry), &md).unwrap();
        let df = md.discount_curve("USD").unwrap().df_to(expiry);
        assert!((c - p - df * (100.0 - 105.0)).abs() < 1e-10);
    }

    #[test]
    fn expired_option_returns_intrinsic_from_settlement() {
        let option = test_option(95, PutOrCall::Call, Date::new(2025, 12, 1));
        let mut md = test_md();
        md.add_settlement_price("UNDERLYING", 101.5);
        let px = price_european(&option, &md).unwrap();
        assert!((px - 6.5).abs() < 1e-12);
    }

    #[test]
    fn expired_otm_option_is_worthless() {
        let option = test_option(110, PutOrCall::Call, Date::new(2025, 12, 1));
        let mut md = test_md();
        md.add_settlement_price("UNDERLYING", 101.5);
        assert_eq!(price_european(&option, &md).unwrap(), 0.0);
    }

    #[test]
    fn missing_vol_surface_errors() {
        let option = test_option(100, PutOrCall::Call, Date::new(2027, 1, 1));
        let valuation = Date::new(2026, 1, 1);
        let mut md = MarketData::new(valuation, valuation.as_of_midnight());
        md.add_market_price("UNDERLYING", 100.0);
        md.add_discount_curve(
            "USD",
            DiscountCurve::flat(valuation, DayCount::Act365Fixed, 0.01),
        );
        let err = price_european(&option, &md).unwrap_err().to_string();
        assert!(err.contains("no vol surface"), "got: {err}");
    }

    #[test]
    fn missing_underlying_price_errors() {
        let option = test_option(100, PutOrCall::Call, Date::new(2027, 1, 1));
        let valuation = Date::new(2026, 1, 1);
        let md = MarketData::new(valuation, valuation.as_of_midnight());
        assert!(price_european(&option, &md).is_err());
    }

    #[test]
    fn american_exercise_rejected() {
        let mut option = test_option(100, PutOrCall::Call, Date::new(2027, 1, 1));
        option.exercise_style = ExerciseStyle::American;
        let err = price_european(&option, &test_md()).unwrap_err().to_string();
        assert!(err.contains("only European exercise"), "got: {err}");
    }

    #[test]
    fn greeks_through_chain() {
        let option = test_option(100, PutOrCall::Call, Date::new(2027, 1, 1));
        let g = greeks_european(&option, &test_md()).unwrap();
        assert!(g.price > 0.0);
        assert!(g.delta > 0.5 && g.delta < 0.6); // ATM call delta ≈ df·N(d1)
        assert!(g.vega > 0.0);
        assert!(g.gamma > 0.0);
        assert!(g.theta < 0.0); // long option decays
    }

    #[test]
    fn greeks_on_expired_option_error() {
        let option = test_option(100, PutOrCall::Call, Date::new(2025, 12, 1));
        let mut md = test_md();
        md.add_settlement_price("UNDERLYING", 100.0);
        assert!(greeks_european(&option, &md).is_err());
    }
}

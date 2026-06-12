use crate::core;
use crate::instruments::bond::Bond;
use crate::pricing::PricingContext;

/// Price a fixed-rate bond as the sum of discounted contractual cash flows.
///
/// The flows (coupons + principal) come from [`Bond::cash_flows`]; this
/// function only discounts the ones that pay after the valuation date.
pub fn price_bond(bond: &Bond, ctx: &dyn PricingContext) -> core::Result<f64> {
    let curve = ctx.discount_curve(&bond.currency.id)?;
    let valuation_date = ctx.valuation_date();

    let mut pv = 0.0;
    for flow in bond.cash_flows() {
        if flow.pay_date > valuation_date {
            pv += decimal_to_f64(flow.amount)? * curve.df_to(flow.pay_date);
        }
    }
    Ok(pv)
}

fn decimal_to_f64(d: rust_decimal::Decimal) -> core::Result<f64> {
    use rust_decimal::prelude::ToPrimitive;
    d.to_f64()
        .ok_or_else(|| core::Error::Pricer(format!("cannot convert Decimal '{}' to f64", d)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cashflows::Frequency;
    use crate::curves::DiscountCurve;
    use crate::dates::Date;
    use crate::dates::daycount::DayCount;
    use crate::dates::rules::DateRule;
    use crate::instruments::Settlement;
    use crate::reference_data::Currency;
    use rust_decimal::Decimal;
    use std::collections::HashMap;
    use std::sync::Arc;

    struct TestContext {
        valuation_date: Date,
        curves: HashMap<String, DiscountCurve>,
    }

    impl PricingContext for TestContext {
        fn as_of(&self) -> crate::dates::Timestamp {
            self.valuation_date.as_of_midnight()
        }
        fn valuation_date(&self) -> Date {
            self.valuation_date
        }
        fn discount_curve(&self, currency: &str) -> crate::core::Result<&DiscountCurve> {
            self.curves
                .get(currency)
                .ok_or_else(|| crate::core::Error::MarketData(format!("no curve for '{}'", currency)))
        }
        fn market_price(&self, _id: &str) -> crate::core::Result<f64> {
            Err(crate::core::Error::MarketData("not implemented".to_string()))
        }
        fn settlement_price(&self, _id: &str) -> crate::core::Result<f64> {
            Err(crate::core::Error::MarketData("not implemented".to_string()))
        }
    }

    fn test_bond(face: u32, coupon: &str, freq: Frequency) -> Bond {
        test_bond_with_dc(face, coupon, freq, DayCount::Act365Fixed)
    }

    fn test_bond_with_dc(face: u32, coupon: &str, freq: Frequency, dc: DayCount) -> Bond {
        let usd = Arc::new(Currency::new("USD", DateRule::Null, DayCount::Act360));
        Bond::new(
            "UST-5Y",
            "US-GOVT",
            usd,
            Settlement::otc(),
            Date::new(2025, 1, 1),
            Date::new(2030, 1, 1),
            Decimal::from(face),
            coupon.parse().unwrap(),
            dc,
            freq,
        )
    }

    fn test_ctx(rate: f64) -> TestContext {
        test_ctx_with_dc(rate, DayCount::Act365Fixed)
    }

    fn test_ctx_with_dc(rate: f64, dc: DayCount) -> TestContext {
        let base = Date::new(2025, 1, 1);
        let mut curves = HashMap::new();
        curves.insert(
            "USD".to_string(),
            DiscountCurve::flat(base, dc, rate),
        );
        TestContext {
            valuation_date: base,
            curves,
        }
    }

    #[test]
    fn zero_coupon_bond_prices_at_df() {
        // Zero coupon: price = face * df(maturity)
        let bond = test_bond(100, "0", Frequency::Annual);
        let ctx = test_ctx(0.05);
        let pv = price_bond(&bond, &ctx).unwrap();
        // df(maturity) from curve directly — exact match
        let expected = ctx.curves["USD"].df_to(Date::new(2030, 1, 1)) * 100.0;
        assert!(
            (pv - expected).abs() < 1e-10,
            "pv={}, expected={}",
            pv,
            expected
        );
    }

    #[test]
    fn zero_rate_prices_at_par() {
        // At zero rates, PV = sum of coupons + face
        let bond = test_bond(100, "0.05", Frequency::SemiAnnual);
        let ctx = test_ctx(0.0);
        let pv = price_bond(&bond, &ctx).unwrap();
        // 5% coupon, semi-annual, 5 years = 10 periods
        // Each coupon ~ face * 0.05 * 0.5 = 2.50 (approx, depends on day count)
        // Total ~ 10 * 2.50 + 100 = 125
        // With Act365Fixed the year fractions won't be exactly 0.5, so allow tolerance
        assert!(
            (pv - 125.0).abs() < 0.50,
            "pv={}, expected ~125",
            pv,
        );
    }

    #[test]
    fn higher_rate_lower_price() {
        let bond = test_bond(100, "0.05", Frequency::SemiAnnual);
        let pv_low = price_bond(&bond, &test_ctx(0.03)).unwrap();
        let pv_high = price_bond(&bond, &test_ctx(0.07)).unwrap();
        assert!(pv_low > pv_high, "lower rate should give higher price");
    }

    #[test]
    fn par_bond_prices_near_par() {
        // A 5% bond discounted at 5% should price near 100
        let bond = test_bond(100, "0.05", Frequency::SemiAnnual);
        let ctx = test_ctx(0.05);
        let pv = price_bond(&bond, &ctx).unwrap();
        // Not exactly 100 due to day count fractions, but close
        assert!(
            (pv - 100.0).abs() < 1.0,
            "5% bond at 5% rate should be near par, got {}",
            pv,
        );
    }

    #[test]
    fn annual_coupon_bond() {
        let bond = test_bond(1000, "0.04", Frequency::Annual);
        let ctx = test_ctx(0.04);
        let pv = price_bond(&bond, &ctx).unwrap();
        assert!(
            (pv - 1000.0).abs() < 10.0,
            "4% annual bond at 4% should be near 1000, got {}",
            pv,
        );
    }

    #[test]
    fn thirty360_par_bond_near_par() {
        // 30/360 semi-annual periods are exactly 0.5, so coupon accrual is clean.
        // Not exactly par because the curve uses continuous compounding while
        // par pricing assumes semi-annual discrete compounding.
        let bond = test_bond_with_dc(100, "0.05", Frequency::SemiAnnual, DayCount::Thirty360);
        let ctx = test_ctx_with_dc(0.05, DayCount::Thirty360);
        let pv = price_bond(&bond, &ctx).unwrap();
        assert!(
            (pv - 100.0).abs() < 0.5,
            "30/360 par bond should be near par, got {}",
            pv,
        );
    }

    #[test]
    fn thirty360_zero_rate_total_coupons() {
        // At zero rates with 30/360: each semi-annual coupon = 100 * 0.06 * 0.5 = 3.00
        // 10 periods + 100 face = 130.00
        let bond = test_bond_with_dc(100, "0.06", Frequency::SemiAnnual, DayCount::Thirty360);
        let ctx = test_ctx_with_dc(0.0, DayCount::Thirty360);
        let pv = price_bond(&bond, &ctx).unwrap();
        assert!(
            (pv - 130.0).abs() < 0.01,
            "30/360 zero-rate 6% bond should be 130.00, got {}",
            pv,
        );
    }

    #[test]
    fn act_act_isda_par_bond() {
        // ActActIsda weights by actual days per year.
        // Not exactly par: continuous compounding vs annual discrete,
        // plus ActActIsda year fractions aren't exactly 1.0 (leap years).
        let bond = test_bond_with_dc(100, "0.05", Frequency::Annual, DayCount::ActActIsda);
        let ctx = test_ctx_with_dc(0.05, DayCount::ActActIsda);
        let pv = price_bond(&bond, &ctx).unwrap();
        assert!(
            (pv - 100.0).abs() < 1.0,
            "ActActIsda par bond should be near par, got {}",
            pv,
        );
    }

    #[test]
    fn act_act_isda_higher_rate_lower_price() {
        let bond = test_bond_with_dc(100, "0.05", Frequency::SemiAnnual, DayCount::ActActIsda);
        let pv_low = price_bond(&bond, &test_ctx_with_dc(0.03, DayCount::ActActIsda)).unwrap();
        let pv_high = price_bond(&bond, &test_ctx_with_dc(0.07, DayCount::ActActIsda)).unwrap();
        assert!(pv_low > pv_high, "lower rate should give higher price (ActActIsda)");
    }
}

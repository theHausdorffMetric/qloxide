use crate::core;
use crate::instruments::bond::Bond;
use crate::pricing::PricingContext;

/// Price a fixed-rate bond as the sum of discounted cash flows.
///
/// Generates coupon dates from issue to maturity, discounts each future
/// coupon payment and the final principal repayment. Coupon periods use
/// the bond's day count convention for accrual fractions.
pub fn price_bond(bond: &Bond, ctx: &dyn PricingContext) -> core::Result<f64> {
    let curve = ctx.discount_curve(&bond.currency.id)?;
    let spot_date = ctx.spot_date();

    let months_per_period = 12 / bond.frequency as i32;
    let coupon_rate_f64 = decimal_to_f64(bond.coupon_rate);
    let face_value_f64 = decimal_to_f64(bond.face_value);

    let mut pv = 0.0;

    // Generate coupon dates forward from issue date
    let mut period_start = bond.issue_date;
    let mut i = 1;
    loop {
        let coupon_date = add_months(bond.issue_date, months_per_period * i);
        let coupon_date = if coupon_date > bond.maturity_date {
            bond.maturity_date
        } else {
            coupon_date
        };

        // Only include future cash flows
        if coupon_date > spot_date {
            let yf = bond.day_count.year_fraction(period_start, coupon_date);
            let coupon = face_value_f64 * coupon_rate_f64 * yf;
            let df = curve.df_to(coupon_date);
            pv += coupon * df;
        }

        if coupon_date >= bond.maturity_date {
            break;
        }
        period_start = coupon_date;
        i += 1;
    }

    // Principal repayment at maturity
    if bond.maturity_date > spot_date {
        let df = curve.df_to(bond.maturity_date);
        pv += face_value_f64 * df;
    }

    Ok(pv)
}

fn decimal_to_f64(d: rust_decimal::Decimal) -> f64 {
    use std::str::FromStr;
    f64::from_str(&d.to_string()).unwrap_or(0.0)
}

/// Add months to a date, clamping to end of month.
fn add_months(date: crate::dates::Date, months: i32) -> crate::dates::Date {
    let total = date.year() as i32 * 12 + (date.month() as i32 - 1) + months;
    let y = (total / 12) as i16;
    let m = (total % 12 + 1) as i8;
    let max_day = days_in_month(y, m);
    let d = date.day().min(max_day);
    crate::dates::Date::new(y, m, d)
}

fn days_in_month(year: i16, month: i8) -> i8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => unreachable!("invalid month: {}", month),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        spot_date: Date,
        curves: HashMap<String, DiscountCurve>,
    }

    impl PricingContext for TestContext {
        fn as_of(&self) -> crate::dates::Timestamp {
            self.spot_date.as_of_midnight()
        }
        fn spot_date(&self) -> Date {
            self.spot_date
        }
        fn discount_curve(&self, currency: &str) -> crate::core::Result<&DiscountCurve> {
            self.curves
                .get(currency)
                .ok_or_else(|| crate::core::Error::MarketData(format!("no curve for '{}'", currency)))
        }
        fn spot(&self, _id: &str) -> crate::core::Result<f64> {
            Err(crate::core::Error::MarketData("not implemented".to_string()))
        }
    }

    fn test_bond(face: u32, coupon: &str, freq: u32) -> Bond {
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
            DayCount::Act365Fixed,
            freq,
        )
    }

    fn test_ctx(rate: f64) -> TestContext {
        let base = Date::new(2025, 1, 1);
        let mut curves = HashMap::new();
        curves.insert(
            "USD".to_string(),
            DiscountCurve::flat(base, DayCount::Act365Fixed, rate),
        );
        TestContext {
            spot_date: base,
            curves,
        }
    }

    #[test]
    fn zero_coupon_bond_prices_at_df() {
        // Zero coupon: price = face * df(maturity)
        let bond = test_bond(100, "0", 1);
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
        let bond = test_bond(100, "0.05", 2);
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
        let bond = test_bond(100, "0.05", 2);
        let pv_low = price_bond(&bond, &test_ctx(0.03)).unwrap();
        let pv_high = price_bond(&bond, &test_ctx(0.07)).unwrap();
        assert!(pv_low > pv_high, "lower rate should give higher price");
    }

    #[test]
    fn par_bond_prices_near_par() {
        // A 5% bond discounted at 5% should price near 100
        let bond = test_bond(100, "0.05", 2);
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
        let bond = test_bond(1000, "0.04", 1);
        let ctx = test_ctx(0.04);
        let pv = price_bond(&bond, &ctx).unwrap();
        assert!(
            (pv - 1000.0).abs() < 10.0,
            "4% annual bond at 4% should be near 1000, got {}",
            pv,
        );
    }
}

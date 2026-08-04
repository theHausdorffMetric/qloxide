use crate::core;
use crate::instruments::payment::Payment;
use crate::pricing::{PricingContext, decimal_to_f64};

/// Price a payment as its discounted amount.
///
/// A payment on or before the valuation date has already settled and
/// contributes zero, matching the flow cutoff in [`super::bond::price_bond`].
pub fn price_payment(payment: &Payment, ctx: &dyn PricingContext) -> core::Result<f64> {
    if payment.pay_date <= ctx.valuation_date() {
        return Ok(0.0);
    }
    let curve = ctx.discount_curve(&payment.currency.id)?;
    Ok(decimal_to_f64(payment.amount)? * curve.df_to(payment.pay_date))
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
            self.curves.get(currency).ok_or_else(|| {
                crate::core::Error::MarketData(format!("no curve for '{}'", currency))
            })
        }
        fn market_price(&self, _id: &str) -> crate::core::Result<f64> {
            Err(crate::core::Error::MarketData(
                "not implemented".to_string(),
            ))
        }
        fn settlement_price(&self, _id: &str) -> crate::core::Result<f64> {
            Err(crate::core::Error::MarketData(
                "not implemented".to_string(),
            ))
        }
        fn vol(&self, _id: &str, _tenor: f64, _moneyness: f64) -> crate::core::Result<f64> {
            Err(crate::core::Error::MarketData(
                "not implemented".to_string(),
            ))
        }
    }

    fn usd() -> Arc<Currency> {
        Arc::new(Currency::new("USD", DateRule::Null, DayCount::Act360))
    }

    fn test_payment(amount: i64, pay_date: Date) -> Payment {
        Payment::new(
            "PAY-1",
            "ACME",
            Decimal::from(amount),
            usd(),
            Settlement::otc(),
            pay_date,
        )
    }

    fn test_ctx(rate: f64) -> TestContext {
        let base = Date::new(2026, 1, 1);
        let mut curves = HashMap::new();
        curves.insert(
            "USD".to_string(),
            DiscountCurve::flat(base, DayCount::Act365Fixed, rate),
        );
        TestContext {
            valuation_date: base,
            curves,
        }
    }

    #[test]
    fn future_payment_prices_at_df() {
        let pay_date = Date::new(2027, 1, 1);
        let payment = test_payment(1_000_000, pay_date);
        let ctx = test_ctx(0.05);
        let pv = price_payment(&payment, &ctx).unwrap();
        let expected = 1_000_000.0 * ctx.curves["USD"].df_to(pay_date);
        assert!(
            (pv - expected).abs() < 1e-10,
            "pv={}, expected={}",
            pv,
            expected
        );
        assert!(pv < 1_000_000.0, "positive rate must discount below par");
    }

    #[test]
    fn negative_amount_prices_negative() {
        let payment = test_payment(-500_000, Date::new(2027, 1, 1));
        let pv = price_payment(&payment, &test_ctx(0.05)).unwrap();
        assert!(pv < 0.0, "pay side must have negative PV, got {}", pv);
    }

    #[test]
    fn settled_payment_prices_at_zero() {
        // On and before the valuation date the cash has moved.
        let on_date = test_payment(1_000_000, Date::new(2026, 1, 1));
        let past = test_payment(1_000_000, Date::new(2025, 6, 1));
        let ctx = test_ctx(0.05);
        assert_eq!(price_payment(&on_date, &ctx).unwrap(), 0.0);
        assert_eq!(price_payment(&past, &ctx).unwrap(), 0.0);
    }

    #[test]
    fn missing_curve_errors() {
        let payment = Payment::new(
            "PAY-2",
            "ACME",
            Decimal::from(100),
            Arc::new(Currency::new("CHF", DateRule::Null, DayCount::Act360)),
            Settlement::otc(),
            Date::new(2027, 1, 1),
        );
        let result = price_payment(&payment, &test_ctx(0.05));
        assert!(result.is_err());
    }

    #[test]
    fn dispatch_reaches_payment_pricer() {
        let payment = test_payment(1_000_000, Date::new(2027, 1, 1));
        let ctx = test_ctx(0.0);
        let pv = crate::pricing::price(&payment, &ctx).unwrap();
        assert!(
            (pv - 1_000_000.0).abs() < 1e-10,
            "zero rate: PV must equal amount, got {}",
            pv
        );
    }
}

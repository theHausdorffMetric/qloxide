pub mod bond;

use crate::core;
use crate::curves::DiscountCurve;
use crate::dates::Date;
use crate::instruments::FinancialInstrument;
use crate::instruments::bond::Bond;
use crate::market_data::MarketData;

/// Market data interface for pricing.
///
/// Provides access to spot prices, discount curves, and (later) vol surfaces.
/// Implementations may be backed by `MarketData` directly or by a caching layer.
pub trait PricingContext {
    fn spot_date(&self) -> Date;
    fn discount_curve(&self, currency: &str) -> core::Result<&DiscountCurve>;
    fn spot(&self, id: &str) -> core::Result<f64>;
}

/// Price a financial instrument.
///
/// Dispatches to the appropriate pricing function based on the concrete
/// instrument type. Model is required only for instruments with optionality.
pub fn price(
    inst: &dyn FinancialInstrument,
    ctx: &dyn PricingContext,
) -> core::Result<f64> {
    let any = inst.as_any();

    // Deterministic instruments — no model needed
    if let Some(b) = any.downcast_ref::<Bond>() {
        return bond::price_bond(b, ctx);
    }

    Err(core::Error::Pricer(format!(
        "no pricer for instrument type '{}'",
        inst.instrument_type()
    )))
}

impl PricingContext for MarketData {
    fn spot_date(&self) -> Date {
        self.spot_date()
    }

    fn discount_curve(&self, currency: &str) -> core::Result<&DiscountCurve> {
        self.discount_curve(currency)
    }

    fn spot(&self, id: &str) -> core::Result<f64> {
        self.spot(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instruments::equity::Equity;
    use crate::instruments::Settlement;

    #[test]
    fn unsupported_instrument_errors() {
        use crate::dates::daycount::DayCount;
        use crate::dates::rules::DateRule;
        use crate::reference_data::Currency;
        use std::sync::Arc;

        let usd = Arc::new(Currency::new("USD", DateRule::Null, DayCount::Act360));
        let equity = Equity::new("AAPL", "APPLE", usd, Settlement::otc());

        let ctx = crate::pricing::tests::EmptyContext {
            spot_date: Date::new(2025, 6, 1),
        };

        let result = price(&equity, &ctx);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no pricer"));
    }

    /// Minimal PricingContext for testing dispatch errors.
    struct EmptyContext {
        spot_date: Date,
    }

    impl PricingContext for EmptyContext {
        fn spot_date(&self) -> Date {
            self.spot_date
        }
        fn discount_curve(&self, currency: &str) -> core::Result<&DiscountCurve> {
            Err(core::Error::MarketData(format!("no curve for '{}'", currency)))
        }
        fn spot(&self, id: &str) -> core::Result<f64> {
            Err(core::Error::MarketData(format!("no spot for '{}'", id)))
        }
    }
}

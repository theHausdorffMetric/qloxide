pub mod bond;
pub mod future;

use crate::core;
use crate::curves::DiscountCurve;
use crate::dates::{Date, Timestamp};
use crate::instruments::FinancialInstrument;
use crate::instruments::bond::Bond;
use crate::instruments::future::Future as FutureInst;
use crate::market_data::MarketData;

/// Market data interface for pricing.
///
/// Provides access to market prices, discount curves, and (later) vol surfaces.
/// Implementations may be backed by `MarketData` directly or by a caching layer.
pub trait PricingContext: Send + Sync {
    fn as_of(&self) -> Timestamp;
    /// The valuation date. No default on purpose: deriving it from
    /// `as_of()` in UTC would roll an evening New York snapshot onto
    /// the next business date. Implementors must choose explicitly.
    fn valuation_date(&self) -> Date;
    fn discount_curve(&self, currency: &str) -> core::Result<&DiscountCurve>;
    fn market_price(&self, id: &str) -> core::Result<f64>;
    fn settlement_price(&self, id: &str) -> core::Result<f64>;
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
    if let Some(f) = any.downcast_ref::<FutureInst>() {
        return future::price_future(f, ctx);
    }

    Err(core::Error::Pricer(format!(
        "no pricer for instrument type '{}'",
        inst.instrument_type()
    )))
}

impl PricingContext for MarketData {
    fn as_of(&self) -> Timestamp {
        self.as_of()
    }

    fn valuation_date(&self) -> Date {
        self.valuation_date()
    }

    fn discount_curve(&self, currency: &str) -> core::Result<&DiscountCurve> {
        self.discount_curve(currency)
    }

    fn market_price(&self, id: &str) -> core::Result<f64> {
        self.market_price(id)
    }

    fn settlement_price(&self, id: &str) -> core::Result<f64> {
        self.settlement_price(id)
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
            valuation_date: Date::new(2025, 6, 1),
        };

        let result = price(&equity, &ctx);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no pricer"));
    }

    /// Minimal PricingContext for testing dispatch errors.
    struct EmptyContext {
        valuation_date: Date,
    }

    impl PricingContext for EmptyContext {
        fn as_of(&self) -> crate::dates::Timestamp {
            self.valuation_date.as_of_midnight()
        }
        fn valuation_date(&self) -> Date {
            self.valuation_date
        }
        fn discount_curve(&self, currency: &str) -> core::Result<&DiscountCurve> {
            Err(core::Error::MarketData(format!("no curve for '{}'", currency)))
        }
        fn market_price(&self, id: &str) -> core::Result<f64> {
            Err(core::Error::MarketData(format!("no market price for '{}'", id)))
        }
        fn settlement_price(&self, id: &str) -> core::Result<f64> {
            Err(core::Error::MarketData(format!("no settlement price for '{}'", id)))
        }
    }
}

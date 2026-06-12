use crate::core;
use crate::instruments::future::Future;
use crate::pricing::PricingContext;

/// Price a future: returns the current market price.
///
/// Uses settlement time when available for precise intraday expiry checks.
/// Falls back to date-level comparison if settlement time cannot be resolved.
/// MTM P&L is computed at the deal level: signed_qty * (mark - trade) * contract_size.
pub fn price_future(future: &Future, ctx: &dyn PricingContext) -> core::Result<f64> {
    let expired = match future.settlement.at_date(future.expiry) {
        Ok(expiry_zoned) => ctx.as_of() > expiry_zoned.timestamp(),
        Err(_) => ctx.valuation_date() > future.expiry,
    };
    if expired {
        return ctx.settlement_price(&future.id);
    }
    ctx.market_price(&future.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curves::DiscountCurve;
    use crate::dates::{Date, Timestamp};
    use crate::dates::daycount::DayCount;
    use crate::dates::rules::DateRule;
    use crate::instruments::Settlement;
    use crate::reference_data::Currency;
    use rust_decimal::Decimal;
    use std::collections::HashMap;
    use std::sync::Arc;

    struct TestContext {
        as_of: Timestamp,
        valuation_date: Date,
        market_prices: HashMap<String, f64>,
        settlements: HashMap<String, f64>,
    }

    impl TestContext {
        fn from_date(valuation_date: Date) -> TestContext {
            TestContext {
                as_of: valuation_date.as_of_midnight(),
                valuation_date,
                market_prices: HashMap::new(),
                settlements: HashMap::new(),
            }
        }
    }

    impl PricingContext for TestContext {
        fn as_of(&self) -> Timestamp {
            self.as_of
        }
        fn valuation_date(&self) -> Date {
            self.valuation_date
        }
        fn discount_curve(&self, currency: &str) -> core::Result<&DiscountCurve> {
            Err(core::Error::MarketData(format!("no curve for '{}'", currency)))
        }
        fn market_price(&self, id: &str) -> core::Result<f64> {
            self.market_prices
                .get(id)
                .copied()
                .ok_or_else(|| core::Error::MarketData(format!("no market price for '{}'", id)))
        }
        fn settlement_price(&self, id: &str) -> core::Result<f64> {
            self.settlements
                .get(id)
                .copied()
                .ok_or_else(|| core::Error::MarketData(format!("no settlement price for '{}'", id)))
        }
        fn vol(&self, id: &str, _tenor: f64, _moneyness: f64) -> core::Result<f64> {
            Err(core::Error::MarketData(format!("no vol surface for '{}'", id)))
        }
    }

    #[test]
    fn future_prices_at_market() {
        let usd = Arc::new(Currency::new("USD", DateRule::Null, DayCount::Act360));
        let future = Future::new(
            "ICE-BRN-K26", "Brent", usd,
            Settlement::new("ICE", "SETTLE", "19:30", "Europe/London", DateRule::Null),
            Date::new(2026, 3, 31),
            Decimal::from(1000),
            "0.01".parse().unwrap(),
        );

        let mut ctx = TestContext::from_date(Date::new(2026, 3, 7));
        ctx.market_prices.insert("ICE-BRN-K26".to_string(), 72.45);

        let px = price_future(&future, &ctx).unwrap();
        assert!((px - 72.45).abs() < 1e-12);
    }

    #[test]
    fn expired_future_uses_settlement_price() {
        let usd = Arc::new(Currency::new("USD", DateRule::Null, DayCount::Act360));
        let future = Future::new(
            "ICE-BRN-K26", "Brent", usd,
            Settlement::new("ICE", "SETTLE", "19:30", "Europe/London", DateRule::Null),
            Date::new(2026, 3, 31),
            Decimal::from(1000),
            "0.01".parse().unwrap(),
        );

        let mut ctx = TestContext::from_date(Date::new(2026, 4, 15));
        ctx.settlements.insert("ICE-BRN-K26".to_string(), 73.10);

        let px = price_future(&future, &ctx).unwrap();
        assert!((px - 73.10).abs() < 1e-12);
    }

    #[test]
    fn on_expiry_date_prices_ok() {
        let usd = Arc::new(Currency::new("USD", DateRule::Null, DayCount::Act360));
        let future = Future::new(
            "ICE-BRN-K26", "Brent", usd,
            Settlement::new("ICE", "SETTLE", "19:30", "Europe/London", DateRule::Null),
            Date::new(2026, 3, 31),
            Decimal::from(1000),
            "0.01".parse().unwrap(),
        );

        let mut ctx = TestContext::from_date(Date::new(2026, 3, 31));
        ctx.market_prices.insert("ICE-BRN-K26".to_string(), 71.90);

        let px = price_future(&future, &ctx).unwrap();
        assert!((px - 71.90).abs() < 1e-12);
    }

    #[test]
    fn missing_market_price_errors() {
        let usd = Arc::new(Currency::new("USD", DateRule::Null, DayCount::Act360));
        let future = Future::new(
            "ICE-BRN-K26", "Brent", usd,
            Settlement::new("ICE", "SETTLE", "19:30", "Europe/London", DateRule::Null),
            Date::new(2026, 3, 31),
            Decimal::from(1000),
            "0.01".parse().unwrap(),
        );

        let ctx = TestContext::from_date(Date::new(2026, 3, 7));
        assert!(price_future(&future, &ctx).is_err());
    }

    #[test]
    fn before_settlement_on_expiry_day_not_expired() {
        // 14:00 UTC on expiry day, settlement is 19:30 London (BST = 18:30 UTC)
        let usd = Arc::new(Currency::new("USD", DateRule::Null, DayCount::Act360));
        let future = Future::new(
            "ICE-BRN-K26", "Brent", usd,
            Settlement::new("ICE", "SETTLE", "19:30", "Europe/London", DateRule::Null),
            Date::new(2026, 3, 31),
            Decimal::from(1000),
            "0.01".parse().unwrap(),
        );

        let mut ctx = TestContext {
            as_of: Timestamp::parse("2026-03-31T14:00:00Z").unwrap(),
            valuation_date: Date::new(2026, 3, 31),
            market_prices: HashMap::new(),
            settlements: HashMap::new(),
        };
        ctx.market_prices.insert("ICE-BRN-K26".to_string(), 72.00);

        let px = price_future(&future, &ctx).unwrap();
        assert!((px - 72.00).abs() < 1e-12);
    }

    #[test]
    fn after_settlement_on_expiry_day_expired() {
        // 20:00 UTC on expiry day, settlement is 19:30 London (BST = 18:30 UTC)
        let usd = Arc::new(Currency::new("USD", DateRule::Null, DayCount::Act360));
        let future = Future::new(
            "ICE-BRN-K26", "Brent", usd,
            Settlement::new("ICE", "SETTLE", "19:30", "Europe/London", DateRule::Null),
            Date::new(2026, 3, 31),
            Decimal::from(1000),
            "0.01".parse().unwrap(),
        );

        let mut ctx = TestContext {
            as_of: Timestamp::parse("2026-03-31T20:00:00Z").unwrap(),
            valuation_date: Date::new(2026, 3, 31),
            market_prices: HashMap::new(),
            settlements: HashMap::new(),
        };
        ctx.settlements.insert("ICE-BRN-K26".to_string(), 71.95);

        let px = price_future(&future, &ctx).unwrap();
        assert!((px - 71.95).abs() < 1e-12);
    }
}

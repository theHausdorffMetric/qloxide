use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::core;
use crate::curves::DiscountCurve;
use crate::dates::{Date, Timestamp};

/// Container for market data: spot prices, discount curves, and (later)
/// forward curves, vol surfaces, fixings.
///
/// Keyed by string identifiers — instrument IDs for spots, currency IDs
/// for discount curves.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MarketData {
    spot_date: Date,
    as_of: Timestamp,
    spots: HashMap<String, f64>,
    #[serde(default)]
    settlement_prices: HashMap<String, f64>,
    discount_curves: HashMap<String, DiscountCurve>,
}

impl MarketData {
    pub fn new(spot_date: Date, as_of: Timestamp) -> MarketData {
        MarketData {
            spot_date,
            as_of,
            spots: HashMap::new(),
            settlement_prices: HashMap::new(),
            discount_curves: HashMap::new(),
        }
    }

    pub fn spot_date(&self) -> Date {
        self.spot_date
    }

    pub fn as_of(&self) -> Timestamp {
        self.as_of
    }

    /// Add a spot price for an instrument.
    pub fn add_spot(&mut self, id: &str, price: f64) {
        self.spots.insert(id.to_string(), price);
    }

    /// Look up a spot price by instrument ID.
    pub fn spot(&self, id: &str) -> core::Result<f64> {
        self.spots
            .get(id)
            .copied()
            .ok_or_else(|| core::Error::MarketData(format!("no spot price for '{}'", id)))
    }

    /// Add a final settlement price for an expired instrument.
    pub fn add_settlement_price(&mut self, id: &str, price: f64) {
        self.settlement_prices.insert(id.to_string(), price);
    }

    /// Look up a final settlement price by instrument ID.
    pub fn settlement_price(&self, id: &str) -> core::Result<f64> {
        self.settlement_prices
            .get(id)
            .copied()
            .ok_or_else(|| core::Error::MarketData(format!("no settlement price for '{}'", id)))
    }

    /// Add a discount curve for a currency.
    pub fn add_discount_curve(&mut self, currency: &str, curve: DiscountCurve) {
        self.discount_curves.insert(currency.to_string(), curve);
    }

    /// Merge another `MarketData` into this one.
    ///
    /// `spot_date` and `as_of` must match. Spots and discount curves are merged;
    /// duplicate keys are an error.
    pub fn merge(&mut self, other: MarketData) -> core::Result<()> {
        if self.spot_date != other.spot_date || self.as_of != other.as_of {
            return Err(core::Error::MarketData(
                format!(
                    "spot_date/as_of mismatch: {}/{} vs {}/{}",
                    self.spot_date, self.as_of, other.spot_date, other.as_of,
                ),
            ));
        }
        for (id, price) in other.spots {
            if self.spots.contains_key(&id) {
                return Err(core::Error::MarketData(
                    format!("duplicate spot '{}'", id),
                ));
            }
            self.spots.insert(id, price);
        }
        for (id, price) in other.settlement_prices {
            if self.settlement_prices.contains_key(&id) {
                return Err(core::Error::MarketData(
                    format!("duplicate settlement price '{}'", id),
                ));
            }
            self.settlement_prices.insert(id, price);
        }
        for (ccy, curve) in other.discount_curves {
            if self.discount_curves.contains_key(&ccy) {
                return Err(core::Error::MarketData(
                    format!("duplicate discount curve '{}'", ccy),
                ));
            }
            self.discount_curves.insert(ccy, curve);
        }
        Ok(())
    }

    /// Look up a discount curve by currency ID.
    pub fn discount_curve(&self, currency: &str) -> core::Result<&DiscountCurve> {
        self.discount_curves
            .get(currency)
            .ok_or_else(|| core::Error::MarketData(format!("no discount curve for '{}'", currency)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dates::daycount::DayCount;

    fn md(date: Date) -> MarketData {
        MarketData::new(date, date.as_of_midnight())
    }

    #[test]
    fn add_and_retrieve_spot() {
        let mut md = md(Date::new(2025, 6, 1));
        md.add_spot("ICE-BRN-Aug25", 72.50);
        assert!((md.spot("ICE-BRN-Aug25").unwrap() - 72.50).abs() < 1e-12);
    }

    #[test]
    fn missing_spot_errors() {
        let md = md(Date::new(2025, 6, 1));
        assert!(md.spot("MISSING").is_err());
    }

    #[test]
    fn add_and_retrieve_discount_curve() {
        let base = Date::new(2025, 1, 1);
        let mut md = md(base);
        let curve = DiscountCurve::flat(base, DayCount::Act365Fixed, 0.05);
        md.add_discount_curve("USD", curve);

        let c = md.discount_curve("USD").unwrap();
        let df = c.df_to(Date::new(2026, 1, 1));
        let expected = (-0.05_f64).exp();
        assert!((df - expected).abs() < 1e-6);
    }

    #[test]
    fn missing_curve_errors() {
        let md = md(Date::new(2025, 6, 1));
        assert!(md.discount_curve("EUR").is_err());
    }

    #[test]
    fn multiple_currencies() {
        let base = Date::new(2025, 1, 1);
        let mut md = md(base);
        md.add_discount_curve("USD", DiscountCurve::flat(base, DayCount::Act365Fixed, 0.05));
        md.add_discount_curve("EUR", DiscountCurve::flat(base, DayCount::Act365Fixed, 0.03));

        let usd_df = md.discount_curve("USD").unwrap().df_to(Date::new(2026, 1, 1));
        let eur_df = md.discount_curve("EUR").unwrap().df_to(Date::new(2026, 1, 1));
        assert!(usd_df < eur_df); // higher rate = lower discount factor
    }
}

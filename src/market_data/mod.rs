use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::core;
use crate::curves::DiscountCurve;
use crate::dates::Date;

/// Container for market data: spot prices, discount curves, and (later)
/// forward curves, vol surfaces, fixings.
///
/// Keyed by string identifiers — instrument IDs for spots, currency IDs
/// for discount curves.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MarketData {
    spot_date: Date,
    spots: HashMap<String, f64>,
    discount_curves: HashMap<String, DiscountCurve>,
}

impl MarketData {
    pub fn new(spot_date: Date) -> MarketData {
        MarketData {
            spot_date,
            spots: HashMap::new(),
            discount_curves: HashMap::new(),
        }
    }

    pub fn spot_date(&self) -> Date {
        self.spot_date
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

    /// Add a discount curve for a currency.
    pub fn add_discount_curve(&mut self, currency: &str, curve: DiscountCurve) {
        self.discount_curves.insert(currency.to_string(), curve);
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

    #[test]
    fn add_and_retrieve_spot() {
        let mut md = MarketData::new(Date::new(2025, 6, 1));
        md.add_spot("ICE-BRN-Aug25", 72.50);
        assert!((md.spot("ICE-BRN-Aug25").unwrap() - 72.50).abs() < 1e-12);
    }

    #[test]
    fn missing_spot_errors() {
        let md = MarketData::new(Date::new(2025, 6, 1));
        assert!(md.spot("MISSING").is_err());
    }

    #[test]
    fn add_and_retrieve_discount_curve() {
        let base = Date::new(2025, 1, 1);
        let mut md = MarketData::new(base);
        let curve = DiscountCurve::flat(base, DayCount::Act365Fixed, 0.05);
        md.add_discount_curve("USD", curve);

        let c = md.discount_curve("USD").unwrap();
        let df = c.df_to(Date::new(2026, 1, 1));
        let expected = (-0.05_f64).exp();
        assert!((df - expected).abs() < 1e-6);
    }

    #[test]
    fn missing_curve_errors() {
        let md = MarketData::new(Date::new(2025, 6, 1));
        assert!(md.discount_curve("EUR").is_err());
    }

    #[test]
    fn multiple_currencies() {
        let base = Date::new(2025, 1, 1);
        let mut md = MarketData::new(base);
        md.add_discount_curve("USD", DiscountCurve::flat(base, DayCount::Act365Fixed, 0.05));
        md.add_discount_curve("EUR", DiscountCurve::flat(base, DayCount::Act365Fixed, 0.03));

        let usd_df = md.discount_curve("USD").unwrap().df_to(Date::new(2026, 1, 1));
        let eur_df = md.discount_curve("EUR").unwrap().df_to(Date::new(2026, 1, 1));
        assert!(usd_df < eur_df); // higher rate = lower discount factor
    }
}

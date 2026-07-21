pub mod store;
pub mod vol;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::core;
use crate::curves::DiscountCurve;
use crate::dates::{Date, Timestamp};

pub use store::MarketStore;
pub use vol::VolSurface;

/// Container for market data: market prices, discount curves, vol surfaces,
/// and (later) forward curves and fixings.
///
/// Keyed by string identifiers — instrument IDs for market_prices and
/// vol_surfaces, currency IDs for discount curves.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MarketData {
    valuation_date: Date,
    as_of: Timestamp,
    /// File-level provenance: origin of the data (e.g. `"ICE"` for exchange
    /// settlements; scenario overlays self-declare `"scenario:…"` so
    /// synthetic data can never impersonate real marks). One file = one
    /// source; a mixed-source file is a merge that happened too early.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    /// File-level lineage: what produced this file (e.g. `"qloxide-ice 0.1.0"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generator: Option<String>,
    // BTreeMaps, not HashMaps: serialization must be byte-deterministic so
    // regenerating an unchanged series reproduces the committed files
    // exactly (I4 — deterministic regeneration).
    market_prices: BTreeMap<String, f64>,
    #[serde(default)]
    settlement_prices: BTreeMap<String, f64>,
    discount_curves: BTreeMap<String, DiscountCurve>,
    #[serde(default)]
    vol_surfaces: BTreeMap<String, VolSurface>,
}

impl MarketData {
    pub fn new(valuation_date: Date, as_of: Timestamp) -> MarketData {
        MarketData {
            valuation_date,
            as_of,
            source: None,
            generator: None,
            market_prices: BTreeMap::new(),
            settlement_prices: BTreeMap::new(),
            discount_curves: BTreeMap::new(),
            vol_surfaces: BTreeMap::new(),
        }
    }

    pub fn valuation_date(&self) -> Date {
        self.valuation_date
    }

    pub fn as_of(&self) -> Timestamp {
        self.as_of
    }

    /// Provenance stamp: where this data originated, if declared.
    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }

    pub fn set_source(&mut self, source: &str) {
        self.source = Some(source.to_string());
    }

    /// Lineage stamp: what produced this data, if declared.
    pub fn generator(&self) -> Option<&str> {
        self.generator.as_deref()
    }

    pub fn set_generator(&mut self, generator: &str) {
        self.generator = Some(generator.to_string());
    }

    /// Add a market price for an instrument.
    pub fn add_market_price(&mut self, id: &str, price: f64) {
        self.market_prices.insert(id.to_string(), price);
    }

    /// Look up a market price by instrument ID.
    pub fn market_price(&self, id: &str) -> core::Result<f64> {
        self.market_prices
            .get(id)
            .copied()
            .ok_or_else(|| core::Error::MarketData(format!("no market price for '{}'", id)))
    }

    /// Add an official settlement price: the settle published for the
    /// instrument at the valuation date, or — for an expired instrument —
    /// its frozen final settle at expiry.
    pub fn add_settlement_price(&mut self, id: &str, price: f64) {
        self.settlement_prices.insert(id.to_string(), price);
    }

    /// All instrument IDs carrying an official settlement price, sorted —
    /// the per-day coverage a series manifest records for completeness
    /// checks without opening every file.
    pub fn settlement_ids(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self.settlement_prices.keys().map(String::as_str).collect();
        ids.sort_unstable();
        ids
    }

    /// Look up an official settlement price by instrument ID.
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

    /// Add a vol surface for an underlying instrument.
    pub fn add_vol_surface(&mut self, id: &str, surface: VolSurface) {
        self.vol_surfaces.insert(id.to_string(), surface);
    }

    /// Look up a vol surface by underlying instrument ID.
    pub fn vol_surface(&self, id: &str) -> core::Result<&VolSurface> {
        self.vol_surfaces
            .get(id)
            .ok_or_else(|| core::Error::MarketData(format!("no vol surface for '{}'", id)))
    }

    /// True if a vol surface exists for the given underlying.
    pub fn has_vol_surface(&self, id: &str) -> bool {
        self.vol_surfaces.contains_key(id)
    }

    /// Merge another `MarketData` into this one.
    ///
    /// `valuation_date` and `as_of` must match. Spots and discount curves are merged;
    /// duplicate keys are an error. Provenance stamps: this side's are kept,
    /// filled from `other` only if absent here. (Scenario-overlay merges will
    /// revisit this — the overlay's `scenario:…` source should then win.)
    pub fn merge(&mut self, other: MarketData) -> core::Result<()> {
        if self.valuation_date != other.valuation_date || self.as_of != other.as_of {
            return Err(core::Error::MarketData(format!(
                "valuation_date/as_of mismatch: {}/{} vs {}/{}",
                self.valuation_date, self.as_of, other.valuation_date, other.as_of,
            )));
        }
        if self.source.is_none() {
            self.source = other.source;
        }
        if self.generator.is_none() {
            self.generator = other.generator;
        }
        for (id, price) in other.market_prices {
            if self.market_prices.contains_key(&id) {
                return Err(core::Error::MarketData(format!(
                    "duplicate market price '{}'",
                    id
                )));
            }
            self.market_prices.insert(id, price);
        }
        for (id, price) in other.settlement_prices {
            if self.settlement_prices.contains_key(&id) {
                return Err(core::Error::MarketData(format!(
                    "duplicate settlement price '{}'",
                    id
                )));
            }
            self.settlement_prices.insert(id, price);
        }
        for (ccy, curve) in other.discount_curves {
            if self.discount_curves.contains_key(&ccy) {
                return Err(core::Error::MarketData(format!(
                    "duplicate discount curve '{}'",
                    ccy
                )));
            }
            self.discount_curves.insert(ccy, curve);
        }
        for (id, surface) in other.vol_surfaces {
            if self.vol_surfaces.contains_key(&id) {
                return Err(core::Error::MarketData(format!(
                    "duplicate vol surface '{}'",
                    id
                )));
            }
            self.vol_surfaces.insert(id, surface);
        }
        Ok(())
    }

    /// Check the contained market data is well-formed (currently: every
    /// vol surface passes [`VolSurface::validate`]). The config loader
    /// calls this after loading/merging, so pricing can rely on total
    /// surface lookups.
    pub fn validate(&self) -> core::Result<()> {
        for (id, surface) in &self.vol_surfaces {
            surface
                .validate()
                .map_err(|e| core::Error::MarketData(format!("'{id}': {e}")))?;
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
    fn add_and_retrieve_market_price() {
        let mut md = md(Date::new(2025, 6, 1));
        md.add_market_price("ICE-BRN-Aug25", 72.50);
        assert!((md.market_price("ICE-BRN-Aug25").unwrap() - 72.50).abs() < 1e-12);
    }

    #[test]
    fn missing_market_price_errors() {
        let md = md(Date::new(2025, 6, 1));
        assert!(md.market_price("MISSING").is_err());
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
        md.add_discount_curve(
            "USD",
            DiscountCurve::flat(base, DayCount::Act365Fixed, 0.05),
        );
        md.add_discount_curve(
            "EUR",
            DiscountCurve::flat(base, DayCount::Act365Fixed, 0.03),
        );

        let usd_df = md
            .discount_curve("USD")
            .unwrap()
            .df_to(Date::new(2026, 1, 1));
        let eur_df = md
            .discount_curve("EUR")
            .unwrap()
            .df_to(Date::new(2026, 1, 1));
        assert!(usd_df < eur_df); // higher rate = lower discount factor
    }
}

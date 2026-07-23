//! The one-file market history (architecture §9).
//!
//! `market.json` *is* the history: a provenance header plus one
//! MarketData-shaped record per trading day, from deal inception to the
//! generation point. The book values against the last day (or `--as-of`),
//! `pnl-series` walks the days, and completeness checks read the records
//! directly — there is no manifest to drift from the data. Non-trading
//! days inside the span are listed in `skipped_days`; a calendar day that
//! is neither a record nor skipped is simply not covered.

use serde::{Deserialize, Serialize};

use crate::core;
use crate::dates::Date;

use super::MarketData;

/// A book's market data across its whole span: header + per-day records.
///
/// Day records are vol-free except where that day's uncleared
/// model-marking needs a surface (§9); provenance lives in the header —
/// one file, one policy, one stamp (I4). Scenario overlay files are
/// one-day histories.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MarketHistory {
    /// Origin of the data (e.g. `"ICE"`); scenario files self-declare
    /// `"scenario:…"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    /// What produced this file (e.g. `"qloxide-ice 0.1.0 …"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generator: Option<String>,
    /// Non-trading days inside the span (weekends, holidays), so coverage
    /// checks can tell "market closed" from "data missing".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    skipped_days: Vec<Date>,
    /// One record per trading day, strictly ascending by valuation date.
    days: Vec<MarketData>,
}

impl MarketHistory {
    /// Build a history from day records (tests and mechanical projection;
    /// generators stamp source/generator afterwards). Call [`validate`]
    /// before trusting the result.
    ///
    /// [`validate`]: MarketHistory::validate
    pub fn new(days: Vec<MarketData>) -> MarketHistory {
        MarketHistory {
            source: None,
            generator: None,
            skipped_days: Vec::new(),
            days,
        }
    }

    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }

    pub fn set_source(&mut self, source: &str) {
        self.source = Some(source.to_string());
    }

    pub fn generator(&self) -> Option<&str> {
        self.generator.as_deref()
    }

    pub fn set_generator(&mut self, generator: &str) {
        self.generator = Some(generator.to_string());
    }

    pub fn skipped_days(&self) -> &[Date] {
        &self.skipped_days
    }

    pub fn set_skipped_days(&mut self, days: Vec<Date>) {
        self.skipped_days = days;
    }

    /// The recorded trading days, in order.
    pub fn days(&self) -> impl Iterator<Item = Date> + '_ {
        self.days.iter().map(|d| d.valuation_date())
    }

    /// The day records themselves, in order.
    pub fn records(&self) -> &[MarketData] {
        &self.days
    }

    pub fn first_day(&self) -> Option<Date> {
        self.days.first().map(|d| d.valuation_date())
    }

    pub fn last_day(&self) -> Option<Date> {
        self.days.last().map(|d| d.valuation_date())
    }

    pub fn len(&self) -> usize {
        self.days.len()
    }

    pub fn is_empty(&self) -> bool {
        self.days.is_empty()
    }

    /// True if the history carries a record for this date.
    pub fn is_trading_day(&self, date: Date) -> bool {
        self.days.iter().any(|d| d.valuation_date() == date)
    }

    /// True if the history accounts for this date at all — as a record or
    /// as an explicitly skipped (non-trading) day.
    pub fn is_known(&self, date: Date) -> bool {
        self.is_trading_day(date) || self.skipped_days.contains(&date)
    }

    /// True if the given day's record carries a settle for the instrument.
    pub fn has_settle(&self, date: Date, id: &str) -> bool {
        self.day(date)
            .map(|d| d.settlement_price(id).is_ok())
            .unwrap_or(false)
    }

    /// The record for a given day.
    pub fn day(&self, date: Date) -> core::Result<&MarketData> {
        self.days
            .iter()
            .find(|d| d.valuation_date() == date)
            .ok_or_else(|| {
                core::Error::MarketData(format!(
                    "no market data for {date} (history covers {})",
                    self.span_label()
                ))
            })
    }

    /// The last record — the default valuation day.
    pub fn last(&self) -> core::Result<&MarketData> {
        self.days
            .last()
            .ok_or_else(|| core::Error::MarketData("market history has no days".to_string()))
    }

    /// Merge a same-shape vols history day-wise into this one: each vols
    /// day must match a recorded day (misalignment is drift, not
    /// enrichment); market days without vols stay vol-free.
    pub fn merge_days(&mut self, other: MarketHistory) -> core::Result<()> {
        let span = self.span_label();
        for day in other.days {
            let date = day.valuation_date();
            let target = self
                .days
                .iter_mut()
                .find(|d| d.valuation_date() == date)
                .ok_or_else(|| {
                    core::Error::MarketData(format!(
                        "vol data day {date} has no matching market day (history covers {span})"
                    ))
                })?;
            target.merge(day)?;
        }
        Ok(())
    }

    /// Well-formedness: at least one day, strictly ascending valuation
    /// dates, and every record's own [`MarketData::validate`].
    pub fn validate(&self) -> core::Result<()> {
        if self.days.is_empty() {
            return Err(core::Error::MarketData(
                "market history has no days".to_string(),
            ));
        }
        for pair in self.days.windows(2) {
            if pair[1].valuation_date() <= pair[0].valuation_date() {
                return Err(core::Error::MarketData(format!(
                    "market history days not strictly ascending: {} then {}",
                    pair[0].valuation_date(),
                    pair[1].valuation_date()
                )));
            }
        }
        for day in &self.days {
            day.validate().map_err(|e| {
                core::Error::MarketData(format!("day {}: {e}", day.valuation_date()))
            })?;
        }
        Ok(())
    }

    fn span_label(&self) -> String {
        match (self.first_day(), self.last_day()) {
            (Some(f), Some(l)) => format!("{f}..{l}"),
            _ => "nothing".to_string(),
        }
    }

    /// The canonical on-disk form: pretty JSON + trailing newline. Every
    /// writer (generators, projections) goes through this so regeneration
    /// stays byte-deterministic (I4).
    pub fn to_canonical_json(&self) -> core::Result<String> {
        let mut s = serde_json::to_string_pretty(self)
            .map_err(|e| core::Error::MarketData(format!("serialize history: {e}")))?;
        s.push('\n');
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(date: &str) -> MarketData {
        let d: Date = serde_json::from_value(serde_json::json!(date)).unwrap();
        MarketData::new(d, d.as_of_midnight())
    }

    #[test]
    fn validate_rejects_empty_and_unordered() {
        assert!(MarketHistory::new(vec![]).validate().is_err());
        let h = MarketHistory::new(vec![day("2026-07-02"), day("2026-07-01")]);
        assert!(h.validate().unwrap_err().to_string().contains("ascending"));
        let h = MarketHistory::new(vec![day("2026-07-01"), day("2026-07-01")]);
        assert!(h.validate().is_err());
        let h = MarketHistory::new(vec![day("2026-07-01"), day("2026-07-02")]);
        assert!(h.validate().is_ok());
    }

    #[test]
    fn day_lookup_and_coverage() {
        let mut h = MarketHistory::new(vec![day("2026-07-01"), day("2026-07-03")]);
        let skipped: Date = serde_json::from_value(serde_json::json!("2026-07-02")).unwrap();
        h.set_skipped_days(vec![skipped]);
        assert!(h.is_trading_day(h.first_day().unwrap()));
        assert!(h.is_known(skipped));
        assert!(!h.is_trading_day(skipped));
        assert_eq!(h.last_day(), h.last().ok().map(|d| d.valuation_date()));
        let missing: Date = serde_json::from_value(serde_json::json!("2026-07-04")).unwrap();
        assert!(!h.is_known(missing));
        assert!(h.day(missing).is_err());
    }

    #[test]
    fn merge_days_requires_alignment() {
        let mut h = MarketHistory::new(vec![day("2026-07-01"), day("2026-07-02")]);
        // Aligned vols day merges…
        let vols = MarketHistory::new(vec![day("2026-07-02")]);
        assert!(h.merge_days(vols).is_ok());
        // …a stray one is drift.
        let stray = MarketHistory::new(vec![day("2026-07-09")]);
        let err = h.merge_days(stray).unwrap_err().to_string();
        assert!(err.contains("no matching market day"), "{err}");
    }

    #[test]
    fn serde_envelope_roundtrip() {
        let mut h = MarketHistory::new(vec![day("2026-07-01")]);
        h.set_source("ICE");
        let json = serde_json::to_string_pretty(&h).unwrap();
        let parsed: MarketHistory = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.source(), Some("ICE"));
        assert_eq!(parsed.len(), 1);
    }
}

//! The tier-1 quote sidecar: raw per-option venue quotes as a
//! first-class data product.
//!
//! `quotes.json` sits beside a book's `market.json`/`vols.json` with the
//! same envelope (header + per-day records) and records what the
//! surface-building pipeline otherwise discards: every venue quote row
//! with its **cleaning disposition** (kept, or dropped with the reason),
//! plus a per-(underlying, day) **conventions block** — the `t`, `f`,
//! `r` under which any vol in the file is interpretable. That block is
//! what makes venue vol-convention mappings testable instead of
//! folklore.
//!
//! Consumers: surface-build validation, RND extraction (prices straight
//! from quotes instead of reverse-engineering surface nodes), arbitrage
//! repair over raw quotes, and convention studies on machines without
//! the venue databases.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::core;
use crate::dates::Date;
use crate::instruments::PutOrCall;

/// A book's raw quotes across its span: header + per-day records, the
/// sibling of [`MarketHistory`](super::MarketHistory) (same provenance
/// rules — one file, one policy, one stamp).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QuoteHistory {
    /// Origin of the data (e.g. `"ICE"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    /// What produced this file (e.g. `"qloxide-ice 0.1.0 …"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generator: Option<String>,
    /// Non-trading days inside the span (weekends, holidays).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    skipped_days: Vec<Date>,
    /// One record per trading day with quotes, strictly ascending.
    days: Vec<QuoteDay>,
}

/// One trading day's quote chains, keyed by underlying id.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QuoteDay {
    pub valuation_date: Date,
    pub chains: BTreeMap<String, QuoteChain>,
}

/// The raw quote rows for one underlying on one day.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuoteChain {
    pub conventions: QuoteConventions,
    /// Every venue quote row, kept and dropped alike, sorted by
    /// (strike, side).
    pub quotes: Vec<Quote>,
}

/// The conventions under which this chain's vols are interpretable: a
/// vol quoted here means "Black-76 with this `t`, this forward, this
/// rate". Venue clocks differ from ours — comparisons must go through
/// this block, never assume a day count.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuoteConventions {
    /// Time to expiry in years under the *generator's* day count.
    pub t: f64,
    /// Underlying forward price.
    pub f: f64,
    /// Flat continuously-compounded discount rate.
    pub r: f64,
}

/// One venue quote row (one side of one strike).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Quote {
    pub strike: f64,
    pub side: PutOrCall,
    /// Venue settlement premium (a discounted price under the
    /// conventions block's `r`).
    pub premium: f64,
    /// Venue-published implied vol, as a decimal (interpretable only
    /// under the *venue's* conventions — see the chain's conventions
    /// block for ours).
    pub published_vol: f64,
    /// Venue-published delta of this leg.
    pub delta: f64,
    /// What the cleaning pipeline did with this row.
    pub disposition: Disposition,
}

/// Cleaning disposition of a quote row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    /// The row survived cleaning (it fed the surface build).
    Kept,
    /// The row was dropped; the reason is part of the record.
    Dropped { reason: String },
}

impl QuoteHistory {
    /// Build a history from day records; generators stamp
    /// source/generator afterwards. Call [`validate`] before trusting
    /// the result.
    ///
    /// [`validate`]: QuoteHistory::validate
    pub fn new(days: Vec<QuoteDay>) -> QuoteHistory {
        QuoteHistory {
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

    /// The day records, in order.
    pub fn records(&self) -> &[QuoteDay] {
        &self.days
    }

    pub fn len(&self) -> usize {
        self.days.len()
    }

    pub fn is_empty(&self) -> bool {
        self.days.is_empty()
    }

    /// The record for one day.
    pub fn day(&self, date: Date) -> core::Result<&QuoteDay> {
        self.days
            .iter()
            .find(|d| d.valuation_date == date)
            .ok_or_else(|| core::Error::MarketData(format!("no quote record for {date}")))
    }

    /// Structural checks: at least one day, strictly ascending dates,
    /// finite conventions, and `t > 0` (expired chains have no
    /// interpretable vols and don't belong in the sidecar).
    pub fn validate(&self) -> core::Result<()> {
        if self.days.is_empty() {
            return Err(core::Error::MarketData(
                "quote history has no days".to_string(),
            ));
        }
        for pair in self.days.windows(2) {
            if pair[1].valuation_date <= pair[0].valuation_date {
                return Err(core::Error::MarketData(format!(
                    "quote days not strictly ascending: {} then {}",
                    pair[0].valuation_date, pair[1].valuation_date
                )));
            }
        }
        for day in &self.days {
            for (underlying, chain) in &day.chains {
                let c = &chain.conventions;
                if !(c.t.is_finite() && c.f.is_finite() && c.r.is_finite()) {
                    return Err(core::Error::MarketData(format!(
                        "{} '{underlying}': non-finite conventions",
                        day.valuation_date
                    )));
                }
                if c.t <= 0.0 {
                    return Err(core::Error::MarketData(format!(
                        "{} '{underlying}': t = {} — expired chains don't belong in the sidecar",
                        day.valuation_date, c.t
                    )));
                }
            }
        }
        Ok(())
    }

    /// Pretty JSON with a trailing newline — the committed-file format.
    pub fn to_canonical_json(&self) -> core::Result<String> {
        let mut s = serde_json::to_string_pretty(self)
            .map_err(|e| core::Error::MarketData(format!("serialize quote history: {e}")))?;
        s.push('\n');
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain(t: f64) -> QuoteChain {
        QuoteChain {
            conventions: QuoteConventions {
                t,
                f: 80.0,
                r: 0.04,
            },
            quotes: vec![Quote {
                strike: 85.0,
                side: PutOrCall::Call,
                premium: 3.19,
                published_vol: 0.6008,
                delta: 0.45,
                disposition: Disposition::Kept,
            }],
        }
    }

    fn day(date: &str, t: f64) -> QuoteDay {
        QuoteDay {
            valuation_date: date.parse().unwrap(),
            chains: BTreeMap::from([("ICE-BRN-U26".to_string(), chain(t))]),
        }
    }

    #[test]
    fn validate_rejects_empty_unordered_and_expired() {
        assert!(QuoteHistory::new(vec![]).validate().is_err());
        let h = QuoteHistory::new(vec![day("2026-07-02", 0.03), day("2026-07-01", 0.03)]);
        assert!(h.validate().unwrap_err().to_string().contains("ascending"));
        let h = QuoteHistory::new(vec![day("2026-07-01", 0.0)]);
        assert!(h.validate().unwrap_err().to_string().contains("expired"));
        QuoteHistory::new(vec![day("2026-07-01", 0.03), day("2026-07-02", 0.03)])
            .validate()
            .unwrap();
    }

    #[test]
    fn disposition_serde_shape() {
        // The file contract: "kept" is a bare string, dropped carries
        // its reason.
        let kept = serde_json::to_value(Disposition::Kept).unwrap();
        assert_eq!(kept, serde_json::json!("kept"));
        let dropped = serde_json::to_value(Disposition::Dropped {
            reason: "|delta| outside band".into(),
        })
        .unwrap();
        assert_eq!(
            dropped,
            serde_json::json!({"dropped": {"reason": "|delta| outside band"}})
        );
        let rt: Disposition = serde_json::from_value(dropped).unwrap();
        assert!(matches!(rt, Disposition::Dropped { .. }));
    }

    #[test]
    fn envelope_round_trips() {
        let mut h = QuoteHistory::new(vec![day("2026-07-16", 0.03)]);
        h.set_source("ICE");
        h.set_generator("test 0.0.0");
        h.set_skipped_days(vec!["2026-07-18".parse().unwrap()]);
        let json = h.to_canonical_json().unwrap();
        let rt: QuoteHistory = serde_json::from_str(&json).unwrap();
        rt.validate().unwrap();
        assert_eq!(rt.source(), Some("ICE"));
        assert_eq!(
            rt.day("2026-07-16".parse().unwrap()).unwrap().chains.len(),
            1
        );
        assert_eq!(rt.to_canonical_json().unwrap(), json);
    }
}

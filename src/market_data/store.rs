//! Manifest-driven market series store: date → market snapshot.
//!
//! A series generator (e.g. qloxide-ice `gen-series`) writes one
//! `market-<date>.json` per trading day plus a `manifest.json` describing
//! the span: which calendar days traded (and their files), which were
//! skipped as non-trading, and — per day — which instrument IDs carry an
//! official settlement price. The manifest's coverage lists make
//! settle-completeness checks O(manifest): no day file is opened until a
//! snapshot is actually needed.
//!
//! ```json
//! {
//!   "from": "2026-07-10",
//!   "through": "2026-07-16",
//!   "source": "ICE",
//!   "generator": "qloxide-ice 0.1.0",
//!   "days": {
//!     "2026-07-10": { "file": "market-2026-07-10.json",
//!                     "settles": ["ICE-BRN-U26", "ICE-BRN-U26-C-88"] }
//!   },
//!   "skipped": ["2026-07-11", "2026-07-12"]
//! }
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::core;
use crate::dates::Date;
use crate::market_data::MarketData;

/// One trading day in the manifest: its snapshot file (relative to the
/// manifest's directory) and the settle coverage recorded for it.
#[derive(Debug, Deserialize)]
struct ManifestDay {
    file: String,
    /// Instrument IDs with an official settle in this day's file.
    /// Required: a manifest without coverage cannot back completeness
    /// checks — regenerate it with a coverage-aware generator.
    settles: Vec<String>,
}

/// Raw manifest shape; dates arrive as strings and are parsed explicitly
/// so a malformed key names itself in the error.
#[derive(Debug, Deserialize)]
struct Manifest {
    days: BTreeMap<String, ManifestDay>,
    #[serde(default)]
    skipped: Vec<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    generator: Option<String>,
}

struct DayEntry {
    file: PathBuf,
    settles: BTreeSet<String>,
}

/// A loaded series manifest: the trading-day calendar, per-day settle
/// coverage, and lazy access to each day's [`MarketData`] snapshot.
pub struct MarketStore {
    days: BTreeMap<Date, DayEntry>,
    skipped: BTreeSet<Date>,
    source: Option<String>,
    generator: Option<String>,
}

impl std::fmt::Debug for MarketStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MarketStore")
            .field("days", &self.days.len())
            .field("skipped", &self.skipped.len())
            .field("source", &self.source)
            .finish()
    }
}

impl MarketStore {
    /// Load a series manifest. Day files are resolved relative to the
    /// manifest's directory but are not opened here — only [`day`]
    /// touches them.
    ///
    /// [`day`]: MarketStore::day
    pub fn load(manifest_path: &Path) -> core::Result<MarketStore> {
        let dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
        let json = std::fs::read_to_string(manifest_path).map_err(|e| {
            core::Error::MarketData(format!("cannot read {}: {e}", manifest_path.display()))
        })?;
        let manifest: Manifest = serde_json::from_str(&json).map_err(|e| {
            core::Error::MarketData(format!(
                "invalid series manifest {}: {e}",
                manifest_path.display()
            ))
        })?;

        let parse_date = |s: &str| -> core::Result<Date> {
            s.parse().map_err(|_| {
                core::Error::MarketData(format!(
                    "series manifest {}: invalid date '{s}'",
                    manifest_path.display()
                ))
            })
        };

        let mut days = BTreeMap::new();
        for (date_str, day) in manifest.days {
            days.insert(
                parse_date(&date_str)?,
                DayEntry {
                    file: dir.join(&day.file),
                    settles: day.settles.into_iter().collect(),
                },
            );
        }
        let mut skipped = BTreeSet::new();
        for s in &manifest.skipped {
            skipped.insert(parse_date(s)?);
        }

        Ok(MarketStore {
            days,
            skipped,
            source: manifest.source,
            generator: manifest.generator,
        })
    }

    /// Trading days in ascending order.
    pub fn days(&self) -> impl Iterator<Item = Date> + '_ {
        self.days.keys().copied()
    }

    pub fn first_day(&self) -> Option<Date> {
        self.days.keys().next().copied()
    }

    pub fn last_day(&self) -> Option<Date> {
        self.days.keys().next_back().copied()
    }

    pub fn len(&self) -> usize {
        self.days.len()
    }

    pub fn is_empty(&self) -> bool {
        self.days.is_empty()
    }

    /// True if the manifest records this date as a trading day.
    pub fn is_trading_day(&self, date: Date) -> bool {
        self.days.contains_key(&date)
    }

    /// True if the manifest accounts for this date at all — as a trading
    /// day or an explicitly skipped (non-trading) one. Dates outside both
    /// are coverage holes.
    pub fn is_known(&self, date: Date) -> bool {
        self.days.contains_key(&date) || self.skipped.contains(&date)
    }

    /// True if the manifest records an official settle for `id` on `date`
    /// (O(manifest) — the day file is not opened).
    pub fn has_settle(&self, date: Date, id: &str) -> bool {
        self.days.get(&date).is_some_and(|d| d.settles.contains(id))
    }

    /// Provenance stamp carried by the manifest, if any.
    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }

    /// Lineage stamp carried by the manifest, if any.
    pub fn generator(&self) -> Option<&str> {
        self.generator.as_deref()
    }

    /// Load one day's snapshot: reads the file, validates it, and checks
    /// its valuation date matches the manifest's calendar.
    pub fn day(&self, date: Date) -> core::Result<MarketData> {
        let entry = self.days.get(&date).ok_or_else(|| {
            core::Error::MarketData(format!("no market snapshot for {date} in the series"))
        })?;
        let json = std::fs::read_to_string(&entry.file).map_err(|e| {
            core::Error::MarketData(format!("cannot read {}: {e}", entry.file.display()))
        })?;
        let md: MarketData = serde_json::from_str(&json).map_err(|e| {
            core::Error::MarketData(format!("invalid market data {}: {e}", entry.file.display()))
        })?;
        md.validate()?;
        if md.valuation_date() != date {
            return Err(core::Error::MarketData(format!(
                "{}: valuation_date {} does not match manifest day {date}",
                entry.file.display(),
                md.valuation_date(),
            )));
        }
        Ok(md)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_manifest(dir: &Path, json: &str) -> PathBuf {
        let path = dir.join("manifest.json");
        std::fs::write(&path, json).unwrap();
        path
    }

    const MANIFEST: &str = r#"{
        "source": "ICE",
        "generator": "test 0.0.0",
        "days": {
            "2026-07-10": { "file": "market-2026-07-10.json",
                            "settles": ["FUT", "OPT"] },
            "2026-07-13": { "file": "market-2026-07-13.json",
                            "settles": ["FUT"] }
        },
        "skipped": ["2026-07-11", "2026-07-12"]
    }"#;

    #[test]
    fn loads_calendar_and_coverage() {
        let dir = tempfile::tempdir().unwrap();
        let store = MarketStore::load(&write_manifest(dir.path(), MANIFEST)).unwrap();

        assert_eq!(store.len(), 2);
        assert_eq!(store.first_day(), Some(Date::new(2026, 7, 10)));
        assert_eq!(store.last_day(), Some(Date::new(2026, 7, 13)));
        assert!(store.is_trading_day(Date::new(2026, 7, 10)));
        assert!(!store.is_trading_day(Date::new(2026, 7, 11)));
        assert!(store.is_known(Date::new(2026, 7, 11))); // skipped weekend
        assert!(!store.is_known(Date::new(2026, 7, 20))); // outside
        assert!(store.has_settle(Date::new(2026, 7, 10), "OPT"));
        assert!(!store.has_settle(Date::new(2026, 7, 13), "OPT"));
        assert_eq!(store.source(), Some("ICE"));
    }

    #[test]
    fn manifest_without_coverage_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"{ "days": { "2026-07-10": "market-2026-07-10.json" } }"#,
        );
        let err = MarketStore::load(&path).unwrap_err().to_string();
        assert!(err.contains("invalid series manifest"), "{err}");
    }

    #[test]
    fn day_checks_valuation_date() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("market-2026-07-10.json"),
            r#"{
                "valuation_date": "2026-07-13",
                "as_of": "2026-07-13T00:00:00Z",
                "market_prices": {},
                "discount_curves": {}
            }"#,
        )
        .unwrap();
        let store = MarketStore::load(&write_manifest(dir.path(), MANIFEST)).unwrap();
        let err = store.day(Date::new(2026, 7, 10)).unwrap_err().to_string();
        assert!(err.contains("does not match manifest day"), "{err}");
    }

    #[test]
    fn missing_day_errors() {
        let dir = tempfile::tempdir().unwrap();
        let store = MarketStore::load(&write_manifest(dir.path(), MANIFEST)).unwrap();
        let err = store.day(Date::new(2026, 7, 11)).unwrap_err().to_string();
        assert!(err.contains("no market snapshot"), "{err}");
    }
}

//! The generator seam (architecture §10): qloxide owns the producer-side
//! contract as code.
//!
//! A [`MarketSource`] answers per-day venue questions — trading calendar,
//! prices, settles (frozen finals for expired instruments), curves,
//! surfaces, provenance stamps — through a span-scoped session (one
//! connection, shared caches). The generic engine [`generate`] does
//! everything venue-agnostic: the inception→through day walk, record
//! assembly, the §9 book/risk split (day records vol-free except where
//! uncleared model-marking needs a surface; every surface into the vols
//! history), header stamping (I1/I4), and validation.
//!
//! The file contract stays the runtime seam (I2): consumers never call a
//! source; this trait standardizes *production*, and load-time validation
//! still polices whatever data shows up. Sealed by convention until the
//! quote sidecar (task 69) or a second venue forces the first revision.

pub mod conformance;
pub mod replay;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::core;
use crate::curves::DiscountCurve;
use crate::dates::Date;
use crate::instruments::{ClearingStatus, EuropeanOption, FinancialInstrument};
use crate::market_data::quotes::{QuoteChain, QuoteDay, QuoteHistory};
use crate::market_data::{MarketData, MarketHistory, VolSurface};
use crate::trades::Deal;

/// A venue/data-source a driver implements: opens span-scoped sessions
/// and identifies its provenance (I1: the policy that builds the data is
/// stamped into the history headers).
pub trait MarketSource {
    type Session<'s>: DaySession
    where
        Self: 's;

    /// Open a session covering `[from, through]` — the place for one
    /// connection over the whole span and shared caches.
    fn open(&self, from: Date, through: Date) -> core::Result<Self::Session<'_>>;

    /// Origin stamp for the history headers (e.g. `"ICE"`).
    fn source_stamp(&self) -> String;

    /// Lineage stamp identifying generator + policy
    /// (e.g. `"qloxide-ice 0.1.0 (vol-source=ice-vol)"`).
    fn generator_stamp(&self) -> String;
}

/// Per-day venue knowledge, answered within a span-scoped session.
/// Every getter returns `Ok(None)` for "no data" — the engine records
/// exactly what the source knows and nothing else.
pub trait DaySession {
    /// Whether the venue traded on this date (drives `skipped_days`).
    fn is_trading_day(&mut self, date: Date) -> core::Result<bool>;

    /// Quoted market price for a live instrument (e.g. a future's anchor).
    fn market_price(&mut self, id: &str, date: Date) -> core::Result<Option<f64>>;

    /// Official settle for a live instrument on `date`.
    fn settle(&mut self, id: &str, date: Date) -> core::Result<Option<f64>>;

    /// Frozen final settle for an instrument expired by `date` (I3: the
    /// value at expiry, passed through unaltered on every later day).
    fn final_settle(&mut self, id: &str, date: Date) -> core::Result<Option<f64>>;

    /// Discount curve for a currency on `date`.
    fn discount_curve(&mut self, currency: &str, date: Date)
    -> core::Result<Option<DiscountCurve>>;

    /// The policy vol surface for an underlying on `date`.
    fn vol_surface(&mut self, underlying: &str, date: Date) -> core::Result<Option<VolSurface>>;

    /// The day's raw quote chains keyed by underlying (the tier-1
    /// sidecar, one [`QuoteChain`] per underlying with live quotes).
    /// Provided: sources without tier-1 data return nothing and no
    /// sidecar is assembled.
    fn quote_chains(&mut self, date: Date) -> core::Result<BTreeMap<String, QuoteChain>> {
        let _ = date;
        Ok(BTreeMap::new())
    }

    /// Venue findings accumulated since the last call (the engine drains
    /// after each day and after the walk).
    fn drain_warnings(&mut self) -> Vec<String> {
        Vec::new()
    }
}

/// Inputs controlling a generation run.
pub struct GenParams {
    /// First calendar day; defaults to the earliest deal date.
    pub from: Option<Date>,
    /// Last calendar day (the evaluation date).
    pub through: Date,
}

/// A generated §9 data plane: the book-tier market history plus, when
/// any day carries a surface, the risk-tier vols history.
#[derive(Debug)]
pub struct Generated {
    pub market: MarketHistory,
    /// `None` for a surface-free book (e.g. futures only).
    pub vols: Option<MarketHistory>,
    /// The tier-1 quote sidecar; `None` when the source has no tier-1
    /// data. Whether it is *written* is the driver's call (venue quote
    /// content may be licensing-gated).
    pub quotes: Option<QuoteHistory>,
    /// Source findings collected during the walk.
    pub warnings: Vec<String>,
}

/// Walk `[from, through]` against a source and assemble the one-file
/// histories. Venue-agnostic by construction: calendar, values, curves
/// and surfaces come from the session; the walk, the record shape, the
/// §9 split, provenance stamping, and validation live here.
pub fn generate(
    source: &impl MarketSource,
    instruments: &[Arc<dyn FinancialInstrument>],
    deals: Option<&[Deal]>,
    params: &GenParams,
) -> core::Result<Generated> {
    let from = params
        .from
        .or_else(|| deals.and_then(|ds| ds.iter().map(|d| d.timestamp.date()).min()))
        .ok_or_else(|| {
            core::Error::Config("generate: no `from` date and no deals to infer it from".into())
        })?;
    if from > params.through {
        return Err(core::Error::Config(format!(
            "generate: span is empty — from {from} is after through {}",
            params.through
        )));
    }

    let mut session = source.open(from, params.through)?;
    let mut warnings: Vec<String> = Vec::new();
    let mut days: Vec<MarketData> = Vec::new();
    let mut quote_days: Vec<QuoteDay> = Vec::new();
    let mut skipped: Vec<Date> = Vec::new();

    let mut date = from;
    while date <= params.through {
        if session.is_trading_day(date)? {
            days.push(assemble_day(&mut session, instruments, date)?);
            let chains = session.quote_chains(date)?;
            if !chains.is_empty() {
                quote_days.push(QuoteDay {
                    valuation_date: date,
                    chains,
                });
            }
        } else {
            skipped.push(date);
        }
        warnings.extend(session.drain_warnings());
        date = date + 1;
    }

    // The §9 split: strip each day's surfaces into the vols history,
    // re-embed the ones the uncleared model-marking rule requires.
    let mut market_days: Vec<MarketData> = Vec::with_capacity(days.len());
    let mut vol_days: Vec<MarketData> = Vec::new();
    for mut md in days {
        let date = md.valuation_date();
        let surfaces = md.take_vol_surfaces();
        if !surfaces.is_empty() {
            let needed = model_marked_underlyings(instruments, deals, date);
            let mut vols_day = MarketData::new(date, date.as_of_midnight());
            for (underlying, surface) in surfaces {
                if needed.contains(underlying.as_str()) {
                    md.add_vol_surface(&underlying, surface.clone());
                }
                vols_day.add_vol_surface(&underlying, surface);
            }
            vol_days.push(vols_day);
        }
        market_days.push(md);
    }

    let stamp = |h: &mut MarketHistory| {
        // Empty stamps stay absent — a header key with an empty string
        // would claim provenance that was never declared.
        let (src, generator) = (source.source_stamp(), source.generator_stamp());
        if !src.is_empty() {
            h.set_source(&src);
        }
        if !generator.is_empty() {
            h.set_generator(&generator);
        }
        h.set_skipped_days(skipped.clone());
    };
    let mut market = MarketHistory::new(market_days);
    stamp(&mut market);
    market.validate()?;
    let vols = if vol_days.is_empty() {
        None
    } else {
        let mut vols = MarketHistory::new(vol_days);
        stamp(&mut vols);
        vols.validate()?;
        Some(vols)
    };
    let quotes = if quote_days.is_empty() {
        None
    } else {
        let mut quotes = QuoteHistory::new(quote_days);
        let (src, generator) = (source.source_stamp(), source.generator_stamp());
        if !src.is_empty() {
            quotes.set_source(&src);
        }
        if !generator.is_empty() {
            quotes.set_generator(&generator);
        }
        quotes.set_skipped_days(skipped.clone());
        quotes.validate()?;
        Some(quotes)
    };
    Ok(Generated {
        market,
        vols,
        quotes,
        warnings,
    })
}

/// One trading day's record: prices for live instruments, settles for
/// live (current) and expired (frozen final) ones, one curve per
/// currency, and the policy surface per live-option underlying (split
/// out of the market record afterwards).
fn assemble_day(
    session: &mut impl DaySession,
    instruments: &[Arc<dyn FinancialInstrument>],
    date: Date,
) -> core::Result<MarketData> {
    let mut md = MarketData::new(date, date.as_of_midnight());

    for inst in instruments {
        let id = inst.id();
        let expired = inst.maturity().is_some_and(|m| m < date);
        if expired {
            if let Some(price) = session.final_settle(id, date)? {
                md.add_settlement_price(id, price);
            }
        } else {
            if let Some(price) = session.market_price(id, date)? {
                md.add_market_price(id, price);
            }
            if let Some(price) = session.settle(id, date)? {
                md.add_settlement_price(id, price);
            }
        }
    }

    let currencies: BTreeSet<String> = instruments
        .iter()
        .map(|i| i.currency().id.clone())
        .collect();
    for ccy in currencies {
        if let Some(curve) = session.discount_curve(&ccy, date)? {
            md.add_discount_curve(&ccy, curve);
        }
    }

    let underlyings: BTreeSet<&str> = instruments
        .iter()
        .filter_map(|inst| {
            let opt = inst.as_any().downcast_ref::<EuropeanOption>()?;
            (date <= opt.expiry).then_some(opt.underlying.as_str())
        })
        .collect();
    for underlying in underlyings {
        if let Some(surface) = session.vol_surface(underlying, date)? {
            md.add_vol_surface(underlying, surface);
        }
    }

    Ok(md)
}

/// Underlyings whose surface a market-history day record must embed: an
/// **uncleared** option on them is alive on `date` (cleared options
/// settle-mark, expired ones mark at their frozen final — neither needs
/// a surface) and, when deals are known, already dealt by `date`
/// (`pnl-series` values a position only from its inception).
pub fn model_marked_underlyings<'a>(
    instruments: &'a [Arc<dyn FinancialInstrument>],
    deals: Option<&[Deal]>,
    date: Date,
) -> BTreeSet<&'a str> {
    let dealt = |id: &str| match deals {
        None => true,
        Some(deals) => deals
            .iter()
            .any(|d| d.instrument_id == id && d.timestamp.date() <= date),
    };
    instruments
        .iter()
        .filter_map(|inst| {
            let opt = inst.as_any().downcast_ref::<EuropeanOption>()?;
            (inst.clearing() == Some(ClearingStatus::Uncleared)
                && date < opt.expiry
                && dealt(&opt.id))
            .then_some(opt.underlying.as_str())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trades::BuySell;

    /// A synthetic weekday-only source: futures settle at 100 + day-of-month,
    /// one Flat surface per option underlying, no curves.
    struct Mock;

    struct MockSession;

    impl MarketSource for Mock {
        type Session<'s> = MockSession;

        fn open(&self, _from: Date, _through: Date) -> core::Result<MockSession> {
            Ok(MockSession)
        }

        fn source_stamp(&self) -> String {
            "MOCK".into()
        }

        fn generator_stamp(&self) -> String {
            "mock 0.0.0".into()
        }
    }

    impl DaySession for MockSession {
        fn is_trading_day(&mut self, date: Date) -> core::Result<bool> {
            Ok(!matches!(date.weekday(), 6 | 7))
        }

        fn market_price(&mut self, id: &str, date: Date) -> core::Result<Option<f64>> {
            Ok(id.starts_with("FUT").then(|| 100.0 + date.day() as f64))
        }

        fn settle(&mut self, id: &str, date: Date) -> core::Result<Option<f64>> {
            Ok((!id.starts_with("MISS")).then(|| 100.0 + date.day() as f64))
        }

        fn final_settle(&mut self, id: &str, _date: Date) -> core::Result<Option<f64>> {
            Ok((!id.starts_with("MISS")).then_some(777.0))
        }

        fn discount_curve(
            &mut self,
            _currency: &str,
            _date: Date,
        ) -> core::Result<Option<DiscountCurve>> {
            Ok(None)
        }

        fn vol_surface(
            &mut self,
            _underlying: &str,
            _date: Date,
        ) -> core::Result<Option<VolSurface>> {
            Ok(Some(VolSurface::Flat { vol: 0.3 }))
        }
    }

    /// Mock with tier-1 data: one quote chain per day on FUT-A.
    struct QuotingMock;

    impl MarketSource for QuotingMock {
        type Session<'s> = QuotingMockSession;

        fn open(&self, _from: Date, _through: Date) -> core::Result<QuotingMockSession> {
            Ok(QuotingMockSession)
        }

        fn source_stamp(&self) -> String {
            "MOCK".into()
        }

        fn generator_stamp(&self) -> String {
            "mock 0.0.0".into()
        }
    }

    struct QuotingMockSession;

    impl DaySession for QuotingMockSession {
        fn is_trading_day(&mut self, date: Date) -> core::Result<bool> {
            MockSession.is_trading_day(date)
        }

        fn market_price(&mut self, id: &str, date: Date) -> core::Result<Option<f64>> {
            MockSession.market_price(id, date)
        }

        fn settle(&mut self, id: &str, date: Date) -> core::Result<Option<f64>> {
            MockSession.settle(id, date)
        }

        fn final_settle(&mut self, id: &str, date: Date) -> core::Result<Option<f64>> {
            MockSession.final_settle(id, date)
        }

        fn discount_curve(
            &mut self,
            _currency: &str,
            _date: Date,
        ) -> core::Result<Option<DiscountCurve>> {
            Ok(None)
        }

        fn vol_surface(
            &mut self,
            _underlying: &str,
            _date: Date,
        ) -> core::Result<Option<VolSurface>> {
            Ok(Some(VolSurface::Flat { vol: 0.3 }))
        }

        fn quote_chains(
            &mut self,
            date: Date,
        ) -> core::Result<BTreeMap<String, crate::market_data::quotes::QuoteChain>> {
            use crate::market_data::quotes::{Disposition, Quote, QuoteChain, QuoteConventions};
            Ok(BTreeMap::from([(
                "FUT-A".to_string(),
                QuoteChain {
                    conventions: QuoteConventions {
                        t: 0.25,
                        f: 100.0 + date.day() as f64,
                        r: 0.04,
                    },
                    quotes: vec![
                        Quote {
                            strike: 100.0,
                            side: crate::instruments::PutOrCall::Call,
                            premium: 2.5,
                            published_vol: 0.3,
                            delta: 0.5,
                            disposition: Disposition::Kept,
                        },
                        Quote {
                            strike: 100.0,
                            side: crate::instruments::PutOrCall::Put,
                            premium: 2.4,
                            published_vol: 0.3,
                            delta: -0.5,
                            disposition: Disposition::Dropped {
                                reason: "same-strike call leg preferred".into(),
                            },
                        },
                    ],
                },
            )]))
        }
    }

    fn future(id: &str, expiry: &str) -> Arc<dyn FinancialInstrument> {
        let json = format!(
            r#"{{
  "type": "Future", "id": "{id}", "underlying": "Brent",
  "currency": {{"id": "USD", "settlement": "Null", "day_count": "Act360"}},
  "settlement": {{"venue": "ICE", "session": "SETTLE", "time": "19:30", "timezone": "Europe/London", "payment_lag": "Null"}},
  "clearing": "cleared", "expiry": "{expiry}",
  "contract_size": "1000", "tick_size": "0.01"
}}"#
        );
        serde_json::from_str(&json).unwrap()
    }

    fn option(
        id: &str,
        underlying: &str,
        clearing: &str,
        expiry: &str,
    ) -> Arc<dyn FinancialInstrument> {
        let json = format!(
            r#"{{
  "type": "EuropeanOption", "id": "{id}", "underlying": "{underlying}", "credit_id": "ICE",
  "currency": {{"id": "USD", "settlement": "Null", "day_count": "Act360"}},
  "settlement": {{"venue": "ICE", "session": "SETTLE", "time": "19:30", "timezone": "Europe/London", "payment_lag": "Null"}},
  "clearing": "{clearing}", "expiry": "{expiry}", "strike": "100",
  "put_or_call": "Call", "exercise_style": "European", "option_settlement": "Cash"
}}"#
        );
        serde_json::from_str(&json).unwrap()
    }

    fn deal(id: &str, instrument: &str, ts: &str) -> Deal {
        Deal {
            id: id.into(),
            instrument_id: instrument.into(),
            direction: BuySell::Buy,
            quantity: crate::Decimal::ONE,
            price: "1".parse().unwrap(),
            timestamp: crate::dates::Timestamp::parse(ts).unwrap(),
            counterparty: "X".into(),
        }
    }

    // 2026-07-06 is a Monday.
    fn params(through: &str) -> GenParams {
        GenParams {
            from: Some(Date::new(2026, 7, 6)),
            through: through.parse().unwrap(),
        }
    }

    #[test]
    fn walk_skips_non_trading_days_and_infers_span_from_deals() {
        let instruments = vec![future("FUT-A", "2026-12-31")];
        let deals = [deal("D1", "FUT-A", "2026-07-06T10:00:00Z")];
        let generated = generate(
            &Mock,
            &instruments,
            Some(&deals),
            &GenParams {
                from: None, // inferred from the deal
                through: "2026-07-13".parse().unwrap(),
            },
        )
        .unwrap();
        let days: Vec<String> = generated.market.days().map(|d| d.to_string()).collect();
        assert_eq!(
            days,
            [
                "2026-07-06",
                "2026-07-07",
                "2026-07-08",
                "2026-07-09",
                "2026-07-10",
                "2026-07-13"
            ]
        );
        let skipped: Vec<String> = generated
            .market
            .skipped_days()
            .iter()
            .map(|d| d.to_string())
            .collect();
        assert_eq!(skipped, ["2026-07-11", "2026-07-12"]);
        assert_eq!(generated.market.source(), Some("MOCK"));
        assert!(
            generated.vols.is_none(),
            "futures-only book grew a vols history"
        );
    }

    #[test]
    fn expiry_freezes_at_the_final_settle() {
        let instruments = vec![future("FUT-A", "2026-07-08")];
        let generated = generate(&Mock, &instruments, None, &params("2026-07-10")).unwrap();
        let day = |d: &str| generated.market.day(d.parse().unwrap()).unwrap();
        // Live through expiry day: daily settle + market price.
        assert_eq!(day("2026-07-08").settlement_price("FUT-A").unwrap(), 108.0);
        assert!(day("2026-07-08").market_price("FUT-A").is_ok());
        // After expiry: frozen final, no market price.
        assert_eq!(day("2026-07-09").settlement_price("FUT-A").unwrap(), 777.0);
        assert!(day("2026-07-09").market_price("FUT-A").is_err());
    }

    #[test]
    fn uncleared_rule_splits_surfaces() {
        // Cleared option: market days vol-free, every surface in vols.
        let instruments = vec![
            future("FUT-A", "2026-12-31"),
            option("OPT-C", "FUT-A", "cleared", "2026-12-01"),
        ];
        let generated = generate(&Mock, &instruments, None, &params("2026-07-07")).unwrap();
        for record in generated.market.records() {
            assert!(
                record.vol_surface_ids().is_empty(),
                "cleared book embeds a surface"
            );
        }
        let vols = generated.vols.expect("surfaces belong in the vols history");
        assert!(vols.records().iter().all(|r| r.has_vol_surface("FUT-A")));
        assert_eq!(vols.source(), Some("MOCK"));

        // Uncleared option: embedded from its deal date onwards only.
        let instruments = vec![
            future("FUT-A", "2026-12-31"),
            option("OPT-U", "FUT-A", "uncleared", "2026-12-01"),
        ];
        let deals = [deal("D1", "OPT-U", "2026-07-07T10:00:00Z")];
        let generated = generate(&Mock, &instruments, Some(&deals), &params("2026-07-07")).unwrap();
        let day = |d: &str| generated.market.day(d.parse().unwrap()).unwrap();
        assert!(
            !day("2026-07-06").has_vol_surface("FUT-A"),
            "embedded before inception"
        );
        assert!(
            day("2026-07-07").has_vol_surface("FUT-A"),
            "not embedded for uncleared marks"
        );
    }

    #[test]
    fn quote_sidecar_assembles_with_stamps() {
        let instruments = vec![
            future("FUT-A", "2026-12-31"),
            option("OPT-C", "FUT-A", "cleared", "2026-12-01"),
        ];
        // The plain mock has no tier-1 data: no sidecar.
        let generated = generate(&Mock, &instruments, None, &params("2026-07-07")).unwrap();
        assert!(
            generated.quotes.is_none(),
            "quote-less source grew a sidecar"
        );

        // The quoting mock gets the same envelope treatment as the
        // market/vols histories: stamps, skipped days, one record per
        // trading day, dispositions preserved.
        let generated = generate(
            &QuotingMock,
            &instruments,
            None,
            &params("2026-07-13"), // Mon..Mon: spans a weekend
        )
        .unwrap();
        let quotes = generated.quotes.expect("tier-1 source => sidecar");
        quotes.validate().unwrap();
        assert_eq!(quotes.len(), generated.market.len());
        assert_eq!(quotes.source(), Some("MOCK"));
        assert_eq!(quotes.generator(), Some("mock 0.0.0"));
        assert_eq!(quotes.skipped_days(), generated.market.skipped_days());
        let day = quotes.day("2026-07-07".parse().unwrap()).unwrap();
        let chain = &day.chains["FUT-A"];
        assert_eq!(chain.conventions.f, 107.0);
        assert_eq!(chain.quotes.len(), 2);
        assert!(matches!(
            chain.quotes[1].disposition,
            crate::market_data::quotes::Disposition::Dropped { .. }
        ));
    }

    #[test]
    fn span_errors() {
        let instruments = vec![future("FUT-A", "2026-12-31")];
        let err = generate(
            &Mock,
            &instruments,
            None,
            &GenParams {
                from: None,
                through: "2026-07-10".parse().unwrap(),
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("no `from`"), "{err}");
        let err = generate(
            &Mock,
            &instruments,
            None,
            &GenParams {
                from: Some("2026-07-11".parse().unwrap()),
                through: "2026-07-10".parse().unwrap(),
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("span is empty"), "{err}");
    }
}

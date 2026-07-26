//! The I5 conformance kit: a generic acceptance suite any
//! [`MarketSource`] implementation runs against, so future drivers stay
//! honest without copying an existing driver's tests.
//!
//! Checks the parts of the contract the engine cannot guarantee by
//! construction — they depend on the *source's* behavior:
//!
//! - **I4 — deterministic regeneration:** two runs over the same span
//!   produce byte-identical histories.
//! - **I3 — settle pass-through:** every settlement price in the output
//!   re-queries to the same value in a fresh session.
//! - **Calendar consistency:** recorded days answer `is_trading_day =
//!   true`, skipped days `false`, in a fresh session.
//! - **The §9 split:** market day records carry no surface beyond the
//!   uncleared model-marking rule; every generated surface appears in
//!   the vols history.
//!
//! Returns findings (empty = pass); the engine's own errors propagate.

use std::sync::Arc;

use crate::core;
use crate::instruments::FinancialInstrument;
use crate::trades::Deal;

use super::{DaySession, GenParams, Generated, MarketSource, generate, model_marked_underlyings};

/// Run the suite. `deals` as for [`generate`].
pub fn check(
    source: &impl MarketSource,
    instruments: &[Arc<dyn FinancialInstrument>],
    deals: Option<&[Deal]>,
    params: &GenParams,
) -> core::Result<Vec<String>> {
    let mut findings: Vec<String> = Vec::new();

    let first = generate(source, instruments, deals, params)?;
    let second = generate(source, instruments, deals, params)?;

    // I4: byte-identical regeneration.
    if canonical(&first)? != canonical(&second)? {
        findings
            .push("I4: two runs over the same span differ — source is not deterministic".into());
    }

    // Calendar consistency + I3 pass-through, in a fresh session.
    let from = first.market.first_day().expect("validated history");
    let mut session = source.open(from, params.through)?;
    for record in first.market.records() {
        let date = record.valuation_date();
        if !session.is_trading_day(date)? {
            findings.push(format!(
                "calendar: recorded day {date} answers is_trading_day = false"
            ));
        }
        for id in record.settlement_ids() {
            let inst = instruments.iter().find(|i| i.id() == id);
            let expired = inst.and_then(|i| i.maturity()).is_some_and(|m| m < date);
            let fresh = if expired {
                session.final_settle(id, date)?
            } else {
                session.settle(id, date)?
            };
            let recorded = record.settlement_price(id).expect("listed id");
            if fresh != Some(recorded) {
                findings.push(format!(
                    "I3: settle '{id}' on {date} re-queries to {fresh:?}, recorded {recorded}"
                ));
            }
        }
    }
    for &date in first.market.skipped_days() {
        if session.is_trading_day(date)? {
            findings.push(format!(
                "calendar: skipped day {date} answers is_trading_day = true"
            ));
        }
    }

    // §9 split.
    for record in first.market.records() {
        let date = record.valuation_date();
        let needed = model_marked_underlyings(instruments, deals, date);
        for id in record.vol_surface_ids() {
            if !needed.contains(id) {
                findings.push(format!(
                    "§9: market day {date} embeds surface '{id}' no position needs"
                ));
            }
            if !vols_has(&first, date, id) {
                findings.push(format!(
                    "§9: embedded surface '{id}' on {date} is missing from the vols history"
                ));
            }
        }
    }

    Ok(findings)
}

fn canonical(generated: &Generated) -> core::Result<String> {
    let mut s = generated.market.to_canonical_json()?;
    if let Some(vols) = &generated.vols {
        s.push_str(&vols.to_canonical_json()?);
    }
    if let Some(quotes) = &generated.quotes {
        s.push_str(&quotes.to_canonical_json()?);
    }
    Ok(s)
}

fn vols_has(generated: &Generated, date: crate::dates::Date, underlying: &str) -> bool {
    generated
        .vols
        .as_ref()
        .and_then(|v| v.day(date).ok())
        .is_some_and(|d| d.has_vol_surface(underlying))
}

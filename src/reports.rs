use std::fmt::Write;

use crate::Decimal;
use crate::config::Portfolio;
use crate::portfolio::{self, ValuedDeal, pnl_totals};

/// Run a named report against a portfolio.
///
/// Returns the report text, or an error if the report name is unknown.
pub fn run(name: &str, portfolio: &Portfolio) -> crate::core::Result<String> {
    match name {
        "instruments" => Ok(instruments(portfolio)),
        "deals" => Ok(deals(portfolio)),
        "positions" => Ok(positions(portfolio)),
        "pnl" => pnl(portfolio),
        "pnl-series" => pnl_series(portfolio),
        _ => Err(crate::core::Error::Config(format!(
            "unknown report '{}'. available: {}",
            name,
            available().join(", ")
        ))),
    }
}

/// Every known report: name paired with a one-line description.
///
/// Single source of truth for [`available`], [`describe`], and the CLI help.
/// Keep each description in sync with the report function's doc comment.
const REPORTS: &[(&str, &str)] = &[
    (
        "instruments",
        "Instrument specification listing (static reference data)",
    ),
    ("deals", "Trade-by-trade deal listing"),
    ("positions", "Compressed net positions per instrument"),
    (
        "pnl",
        "P&L on raw deals, split into realized/unrealized with totals",
    ),
    (
        "pnl-series",
        "Daily portfolio P&L trajectory over the market series (composition-aware)",
    ),
];

/// List all known report names.
pub fn available() -> Vec<&'static str> {
    REPORTS.iter().map(|(name, _)| *name).collect()
}

/// One-line description for a report name, or `None` if unknown.
pub fn describe(name: &str) -> Option<&'static str> {
    REPORTS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, desc)| *desc)
}

/// `name -> description` pairs for every report. Drives CLI help.
pub fn descriptions() -> &'static [(&'static str, &'static str)] {
    REPORTS
}

/// Instrument specification listing — pure reference data, no valuation
/// (marks live in `pnl`; needs no market data at all).
pub fn instruments(portfolio: &Portfolio) -> String {
    use crate::instruments::{EuropeanOption, Future, PutOrCall};

    let mut out = String::new();

    writeln!(out, "=== Instruments ({}) ===", portfolio.instruments.len()).unwrap();
    writeln!(
        out,
        "{:<16} {:<14} {:<16} {:<4} {:<9} {:<3} {:>8}  {:>10}",
        "ID", "Type", "Underlying", "Ccy", "Clearing", "P/C", "Strike", "Expiry"
    )
    .unwrap();
    writeln!(out, "{:-<88}", "").unwrap();

    let mut ids: Vec<&String> = portfolio.instruments.keys().collect();
    ids.sort();

    for id in &ids {
        let inst = &portfolio.instruments[*id];
        let any = inst.as_any();

        let underlying = any
            .downcast_ref::<Future>()
            .map(|f| f.underlying.as_str())
            .or_else(|| {
                any.downcast_ref::<EuropeanOption>()
                    .map(|o| o.underlying.as_str())
            })
            .unwrap_or("-");
        let (side, strike) = match any.downcast_ref::<EuropeanOption>() {
            Some(o) => (
                match o.put_or_call {
                    PutOrCall::Call => "C",
                    PutOrCall::Put => "P",
                },
                o.strike.to_string(),
            ),
            None => ("-", "-".to_string()),
        };
        let clearing = inst
            .clearing()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "-".to_string());
        let expiry = inst
            .maturity()
            .map(|m| m.to_string())
            .unwrap_or_else(|| "-".to_string());

        writeln!(
            out,
            "{:<16} {:<14} {:<16} {:<4} {:<9} {:<3} {:>8}  {:>10}",
            id,
            inst.instrument_type(),
            underlying,
            inst.currency().id,
            clearing,
            side,
            strike,
            expiry
        )
        .unwrap();
    }

    out
}

/// Deal listing.
pub fn deals(portfolio: &Portfolio) -> String {
    let mut out = String::new();

    writeln!(out, "=== Deals ({}) ===", portfolio.deals.len()).unwrap();
    writeln!(
        out,
        "{:<10} {:<16} {:>5} {:>5}  {:>8}  Timestamp",
        "Deal", "Instrument", "Side", "Qty", "Price"
    )
    .unwrap();
    writeln!(out, "{:-<68}", "").unwrap();

    for deal in &portfolio.deals {
        writeln!(
            out,
            "{:<10} {:<16} {:>5} {:>5}  {:>8}  {}",
            deal.id,
            deal.instrument_id,
            format!("{:?}", deal.direction),
            deal.quantity,
            deal.price,
            deal.timestamp
        )
        .unwrap();
    }

    out
}

/// Compressed net positions per instrument.
pub fn positions(portfolio: &Portfolio) -> String {
    let compressed = portfolio::compress(&portfolio.deals);
    let mut out = String::new();

    let active: Vec<_> = compressed
        .iter()
        .filter(|p| p.quantity > Decimal::ZERO)
        .collect();
    let flat: Vec<_> = compressed
        .iter()
        .filter(|p| p.quantity == Decimal::ZERO)
        .collect();

    writeln!(out, "=== Positions ({} instruments) ===", compressed.len()).unwrap();
    writeln!(
        out,
        "{:<16} {:>5} {:>5}  {:>10}",
        "Instrument", "Side", "Qty", "Avg Price"
    )
    .unwrap();
    writeln!(out, "{:-<44}", "").unwrap();

    for pos in &active {
        writeln!(
            out,
            "{:<16} {:>5} {:>5}  {:>10}",
            pos.instrument_id,
            format!("{:?}", pos.direction),
            pos.quantity,
            pos.avg_price.round_dp(2)
        )
        .unwrap();
    }

    if !flat.is_empty() {
        writeln!(out).unwrap();
        for pos in &flat {
            writeln!(out, "{:<16}  flat", pos.instrument_id).unwrap();
        }
    }

    out
}

/// P&L report on raw deals with realized/unrealized split.
///
/// The one report that values the book — it requires market data.
pub fn pnl(portfolio: &Portfolio) -> crate::core::Result<String> {
    if portfolio.market_data.is_none() {
        return Err(crate::core::Error::Config(
            "report 'pnl' requires market_data in the config".to_string(),
        ));
    }
    // Integrity errors block valuation — an official P&L over a book whose
    // settle history is known-incomplete would be silently wrong.
    if !portfolio.integrity_errors.is_empty() {
        return Err(crate::core::Error::Config(format!(
            "report 'pnl' refused: {} integrity error(s), first: {}",
            portfolio.integrity_errors.len(),
            portfolio.integrity_errors[0],
        )));
    }
    let valued = portfolio::valuate(&portfolio.deals, portfolio);
    Ok(format_pnl(&valued, portfolio.deals.len()))
}

/// Daily portfolio P&L trajectory over the market series.
///
/// For each trading day in [earliest deal date, evaluation date], values
/// the book *as composed on that day* — a deal contributes only from its
/// inception date — and reports realized/unrealized/total plus the daily
/// change (the variation-margin view; day changes telescope to the final
/// total). This is the true P&L history; valuing today's full book
/// against a historical day (`--config series/day-<date>.toml`) is a
/// what-if, not history.
pub fn pnl_series(portfolio: &Portfolio) -> crate::core::Result<String> {
    let store = portfolio.market_series.as_ref().ok_or_else(|| {
        crate::core::Error::Config(
            "report 'pnl-series' requires market_series in the config".to_string(),
        )
    })?;
    if !portfolio.integrity_errors.is_empty() {
        return Err(crate::core::Error::Config(format!(
            "report 'pnl-series' refused: {} integrity error(s), first: {}",
            portfolio.integrity_errors.len(),
            portfolio.integrity_errors[0],
        )));
    }

    let mut out = String::new();
    let Some(start) = portfolio.deals.iter().map(|d| d.timestamp.date()).min() else {
        writeln!(out, "=== P&L series (no deals) ===").unwrap();
        return Ok(out);
    };
    let eval = portfolio
        .market_data
        .as_ref()
        .map(|md| md.valuation_date())
        .or_else(|| store.last_day())
        .ok_or_else(|| crate::core::Error::Config("market series is empty".to_string()))?;

    let days: Vec<_> = store.days().filter(|d| *d >= start && *d <= eval).collect();
    writeln!(
        out,
        "=== P&L series ({} trading days, {} deals) ===",
        days.len(),
        portfolio.deals.len()
    )
    .unwrap();
    writeln!(
        out,
        "{:<10} {:>5}  {:>11}  {:>11}  {:>11}  {:>11}",
        "Date", "Deals", "Realized", "Unrealized", "Total", "Day change"
    )
    .unwrap();
    writeln!(out, "{:-<66}", "").unwrap();

    let mut prev = Decimal::ZERO;
    let mut unpriced_days: Vec<(crate::dates::Date, usize)> = Vec::new();
    for date in days {
        let md = store.day(date)?;
        // The book as it existed on this day: deals struck by then.
        let active: Vec<_> = portfolio
            .deals
            .iter()
            .filter(|d| d.timestamp.date() <= date)
            .cloned()
            .collect();
        let valued = portfolio::valuate_at(&active, &portfolio.instruments, &md);
        let unpriced = valued.iter().filter(|v| v.valuation.is_err()).count();
        if unpriced > 0 {
            unpriced_days.push((date, unpriced));
        }
        let (realized, unrealized) = pnl_totals(&valued);
        let total = realized + unrealized;
        writeln!(
            out,
            "{:<10} {:>5}  {:>11}  {:>11}  {:>11}  {:>11}",
            date.to_string(),
            active.len(),
            format!("{:.2}", realized.round_dp(2)),
            format!("{:.2}", unrealized.round_dp(2)),
            format!("{:.2}", total.round_dp(2)),
            format!("{:.2}", (total - prev).round_dp(2)),
        )
        .unwrap();
        prev = total;
    }
    for (date, n) in unpriced_days {
        writeln!(
            out,
            "WARNING: {date}: {n} deal(s) unpriced and excluded from that day's totals"
        )
        .unwrap();
    }

    out.push('\n');
    Ok(out)
}

fn format_pnl(valued: &[ValuedDeal], deal_count: usize) -> String {
    let mut out = String::new();

    writeln!(out, "=== P&L ({} deals) ===", deal_count).unwrap();
    writeln!(
        out,
        "{:<10} {:<16} {:>5} {:>5}  {:>8}  {:>8}  {:<6}  {:>10}",
        "Deal", "Instrument", "Side", "Qty", "Trade", "Mark", "Source", "P&L"
    )
    .unwrap();
    writeln!(out, "{:-<90}", "").unwrap();

    let mut unpriced = 0;
    for v in valued {
        match &v.valuation {
            Ok(val) => {
                let label = if val.realized {
                    "realized"
                } else {
                    "unrealized"
                };
                writeln!(
                    out,
                    "{:<10} {:<16} {:>5} {:>5}  {:>8}  {:>8}  {:<6}  {:>10}  {}",
                    v.deal.id,
                    v.deal.instrument_id,
                    format!("{:?}", v.deal.direction),
                    v.deal.quantity,
                    v.deal.price,
                    val.mark.round_dp(4),
                    val.source.to_string(),
                    format!("{:.2}", val.pnl.round_dp(2)),
                    label
                )
                .unwrap();
            }
            Err(e) => {
                unpriced += 1;
                writeln!(
                    out,
                    "{:<10} {:<16} {:>5} {:>5}  {:>8}  UNPRICED: {}",
                    v.deal.id,
                    v.deal.instrument_id,
                    format!("{:?}", v.deal.direction),
                    v.deal.quantity,
                    v.deal.price,
                    e
                )
                .unwrap();
            }
        }
    }

    let (realized, unrealized) = pnl_totals(valued);
    writeln!(out, "{:-<90}", "").unwrap();
    writeln!(
        out,
        "{:>70} {:>10}",
        "Realized:",
        format!("{:.2}", realized.round_dp(2))
    )
    .unwrap();
    writeln!(
        out,
        "{:>70} {:>10}",
        "Unrealized:",
        format!("{:.2}", unrealized.round_dp(2))
    )
    .unwrap();
    writeln!(
        out,
        "{:>70} {:>10}",
        "Total:",
        format!("{:.2}", (realized + unrealized).round_dp(2))
    )
    .unwrap();
    if unpriced > 0 {
        writeln!(
            out,
            "WARNING: {unpriced} deal(s) could not be priced and are excluded from totals"
        )
        .unwrap();
    }

    out
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use crate::config::Portfolio;
    use crate::dates::rules::DateRule;
    use crate::dates::{Date, Timestamp};
    use crate::instruments::{Clearing, FinancialInstrument, Future, Settlement};
    use crate::market_data::MarketStore;
    use crate::reference_data::Currency;
    use crate::trades::{BuySell, Deal};
    use rust_decimal::Decimal;

    fn day_json(date: &str, settle: f64) -> String {
        format!(
            r#"{{ "valuation_date": "{date}", "as_of": "{date}T00:00:00Z",
                 "market_prices": {{}}, "settlement_prices": {{ "FUT": {settle} }},
                 "discount_curves": {{}} }}"#
        )
    }

    fn deal(id: &str, price: &str, ts: &str) -> Deal {
        Deal {
            id: id.to_string(),
            instrument_id: "FUT".to_string(),
            direction: BuySell::Buy,
            quantity: Decimal::ONE,
            price: price.parse().unwrap(),
            timestamp: Timestamp::parse(ts).unwrap(),
            counterparty: "TEST".to_string(),
        }
    }

    /// Trajectory over a 4-day series: a second deal enters on day 2
    /// (composition-aware), day changes telescope to the final total, and
    /// the future's 03-04 expiry flips the P&L to realized on 03-05 at the
    /// frozen final settle (expiry cash-settlement).
    #[test]
    fn pnl_series_trajectory_composition_and_expiry() {
        let dir = tempfile::tempdir().unwrap();
        for (date, settle) in [
            ("2026-03-02", 101.0),
            ("2026-03-03", 102.0),
            ("2026-03-04", 105.0),
            ("2026-03-05", 105.0), // frozen final settle post-expiry
        ] {
            std::fs::write(
                dir.path().join(format!("market-{date}.json")),
                day_json(date, settle),
            )
            .unwrap();
        }
        let day =
            |d: &str| format!(r#""{d}": {{ "file": "market-{d}.json", "settles": ["FUT"] }}"#);
        std::fs::write(
            dir.path().join("manifest.json"),
            format!(
                r#"{{ "days": {{ {}, {}, {}, {} }} }}"#,
                day("2026-03-02"),
                day("2026-03-03"),
                day("2026-03-04"),
                day("2026-03-05"),
            ),
        )
        .unwrap();

        let usd = Arc::new(Currency::new(
            "USD",
            DateRule::Null,
            crate::dates::daycount::DayCount::Act360,
        ));
        let fut = Future::new(
            "FUT",
            "Brent",
            usd,
            Settlement::new("ICE", "SETTLE", "19:30", "Europe/London", DateRule::Null),
            Clearing::Ice,
            Date::new(2026, 3, 4),
            Decimal::ONE,
            "0.01".parse().unwrap(),
        );
        let mut instruments: HashMap<String, Arc<dyn FinancialInstrument>> = HashMap::new();
        instruments.insert("FUT".to_string(), Arc::new(fut));

        let portfolio = Portfolio {
            instruments,
            deals: vec![
                deal("D1", "100", "2026-03-02T10:00:00Z"),
                deal("D2", "102", "2026-03-03T10:00:00Z"),
            ],
            market_data: None,
            market_series: Some(MarketStore::load(&dir.path().join("manifest.json")).unwrap()),
            warnings: vec![],
            integrity_errors: vec![],
            reports: vec![],
        };

        let out = super::pnl_series(&portfolio).unwrap();
        let row = |date: &str| -> Vec<String> {
            out.lines()
                .find(|l| l.starts_with(date))
                .unwrap_or_else(|| panic!("no row for {date} in:\n{out}"))
                .split_whitespace()
                .map(str::to_string)
                .collect()
        };
        // date, deals, realized, unrealized, total, day change
        assert_eq!(
            row("2026-03-02"),
            ["2026-03-02", "1", "0.00", "1.00", "1.00", "1.00"]
        );
        assert_eq!(
            row("2026-03-03"),
            ["2026-03-03", "2", "0.00", "2.00", "2.00", "1.00"]
        );
        assert_eq!(
            row("2026-03-04"),
            ["2026-03-04", "2", "0.00", "8.00", "8.00", "6.00"]
        );
        // Post-expiry: cash-settled at the frozen final settle — realized.
        assert_eq!(
            row("2026-03-05"),
            ["2026-03-05", "2", "8.00", "0.00", "8.00", "0.00"]
        );
        assert!(!out.contains("WARNING"), "{out}");
    }

    #[test]
    fn pnl_series_requires_series() {
        let portfolio = Portfolio {
            instruments: HashMap::new(),
            deals: vec![],
            market_data: None,
            market_series: None,
            warnings: vec![],
            integrity_errors: vec![],
            reports: vec![],
        };
        let err = super::pnl_series(&portfolio).unwrap_err().to_string();
        assert!(err.contains("requires market_series"), "{err}");
    }
}

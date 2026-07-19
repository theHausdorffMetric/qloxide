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

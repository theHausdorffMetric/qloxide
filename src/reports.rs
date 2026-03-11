use std::fmt::Write;

use crate::Decimal;
use crate::config::Portfolio;
use crate::instruments::future::Future;
use crate::portfolio::{self, ValuedDeal, pnl_totals};

/// Run a named report against a portfolio.
///
/// Returns the report text, or an error if the report name is unknown.
pub fn run(name: &str, portfolio: &Portfolio) -> crate::core::Result<String> {
    match name {
        "instruments" => Ok(instruments(portfolio)),
        "deals" => Ok(deals(portfolio)),
        "positions" => Ok(positions(portfolio)),
        "pnl" => Ok(pnl(portfolio)),
        _ => Err(crate::core::Error::Config(
            format!("unknown report '{}'. available: {}", name, available().join(", ")),
        )),
    }
}

/// List all known report names.
pub fn available() -> &'static [&'static str] {
    &["instruments", "deals", "positions", "pnl"]
}

/// Instrument listing with spot/settlement prices and status.
pub fn instruments(portfolio: &Portfolio) -> String {
    let mut out = String::new();
    let md = &portfolio.market_data;

    writeln!(out, "=== Instruments ({}) ===", portfolio.instruments.len()).unwrap();
    writeln!(out, "{:<16} {:>10}  {:>10}  Status", "ID", "Expiry", "Price").unwrap();
    writeln!(out, "{:-<56}", "").unwrap();

    let mut ids: Vec<&String> = portfolio.instruments.keys().collect();
    ids.sort();

    for id in &ids {
        let inst = &portfolio.instruments[*id];

        let expiry = inst.as_any().downcast_ref::<Future>()
            .map(|f| f.expiry.to_string())
            .unwrap_or_else(|| "-".to_string());

        let expired = expiry != "-" && md.spot_date().to_string() > expiry;

        let (price, status) = if expired {
            let p = md.settlement_price(id)
                .map(|s| format!("{:.2}", s))
                .unwrap_or_else(|_| "N/A".to_string());
            (p, "settled")
        } else {
            let p = md.spot(id)
                .map(|s| format!("{:.2}", s))
                .unwrap_or_else(|_| "N/A".to_string());
            (p, "active")
        };

        writeln!(out, "{:<16} {:>10}  {:>10}  {}", id, expiry, price, status).unwrap();
    }

    out
}

/// Deal listing.
pub fn deals(portfolio: &Portfolio) -> String {
    let mut out = String::new();

    writeln!(out, "=== Deals ({}) ===", portfolio.deals.len()).unwrap();
    writeln!(out, "{:<10} {:<16} {:>5} {:>5}  {:>8}  Timestamp",
        "Deal", "Instrument", "Side", "Qty", "Price").unwrap();
    writeln!(out, "{:-<68}", "").unwrap();

    for deal in &portfolio.deals {
        writeln!(out, "{:<10} {:<16} {:>5} {:>5}  {:>8}  {}",
            deal.id, deal.instrument_id,
            format!("{:?}", deal.direction), deal.quantity,
            deal.price, deal.timestamp).unwrap();
    }

    out
}

/// Compressed net positions per instrument.
pub fn positions(portfolio: &Portfolio) -> String {
    let compressed = portfolio::compress(&portfolio.deals);
    let mut out = String::new();

    let active: Vec<_> = compressed.iter().filter(|d| d.quantity > Decimal::ZERO).collect();
    let flat: Vec<_> = compressed.iter().filter(|d| d.quantity == Decimal::ZERO).collect();

    writeln!(out, "=== Positions ({} instruments) ===", compressed.len()).unwrap();
    writeln!(out, "{:<16} {:>5} {:>5}  {:>10}",
        "Instrument", "Side", "Qty", "Avg Price").unwrap();
    writeln!(out, "{:-<44}", "").unwrap();

    for deal in &active {
        writeln!(out, "{:<16} {:>5} {:>5}  {:>10}",
            deal.instrument_id,
            format!("{:?}", deal.direction), deal.quantity,
            deal.price.round_dp(2)).unwrap();
    }

    if !flat.is_empty() {
        writeln!(out).unwrap();
        for deal in &flat {
            writeln!(out, "{:<16}  flat", deal.instrument_id).unwrap();
        }
    }

    out
}

/// P&L report on raw deals with realized/unrealized split.
pub fn pnl(portfolio: &Portfolio) -> String {
    let valued = portfolio::valuate(&portfolio.deals, portfolio);
    format_pnl(&valued, portfolio.deals.len())
}

fn format_pnl(valued: &[ValuedDeal], deal_count: usize) -> String {
    let mut out = String::new();

    writeln!(out, "=== P&L ({} deals) ===", deal_count).unwrap();
    writeln!(out, "{:<10} {:<16} {:>5} {:>5}  {:>8}  {:>8}  {:>10}",
        "Deal", "Instrument", "Side", "Qty", "Trade", "Mark", "P&L").unwrap();
    writeln!(out, "{:-<82}", "").unwrap();

    for v in valued {
        let label = if v.realized { "realized" } else { "unrealized" };
        writeln!(out, "{:<10} {:<16} {:>5} {:>5}  {:>8}  {:>8}  {:>10}  {}",
            v.deal.id, v.deal.instrument_id,
            format!("{:?}", v.deal.direction), v.deal.quantity,
            v.deal.price, v.mark, v.pnl, label).unwrap();
    }

    let (realized, unrealized) = pnl_totals(valued);
    writeln!(out, "{:-<82}", "").unwrap();
    writeln!(out, "{:>62} {:>10}", "Realized:", realized).unwrap();
    writeln!(out, "{:>62} {:>10}", "Unrealized:", unrealized).unwrap();
    writeln!(out, "{:>62} {:>10}", "Total:", realized + unrealized).unwrap();

    out
}

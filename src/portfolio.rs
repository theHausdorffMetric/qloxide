use std::collections::HashMap;
use std::sync::Arc;

use crate::Decimal;
use crate::config::Portfolio;
use crate::core;
use crate::instruments::FinancialInstrument;
use crate::instruments::future::Future;
use crate::instruments::option::EuropeanOption;
use crate::pricing;
use crate::trades::{BuySell, Deal};

/// A deal enriched with mark-to-market valuation — or the reason it
/// could not be priced. Every input deal produces exactly one
/// `ValuedDeal`; pricing failures are never silently dropped.
pub struct ValuedDeal {
    pub deal: Deal,
    pub valuation: core::Result<Valuation>,
}

/// Successful mark-to-market result for one deal.
#[derive(Debug)]
pub struct Valuation {
    pub mark: Decimal,
    pub pnl: Decimal,
    pub realized: bool,
    /// Where the mark came from — the pnl report's Source column.
    pub source: MarkSource,
}

/// Provenance of an official mark. The epistemic grade of a valuation:
/// a settle is observed, a model price is computed, a proxy borrows an
/// observation from a configured twin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkSource {
    /// Official settlement price — audit-grade, no model in the path.
    Settle,
    /// Model price (uncleared instruments, or types without settles).
    Model,
    /// A cleared twin's settlement price, adopted by explicit per-book
    /// config for an uncleared lookalike — an observed price, but of a
    /// different contract; books-grade only by that book's policy.
    Proxy,
}

impl std::fmt::Display for MarkSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            MarkSource::Settle => "settle",
            MarkSource::Model => "model",
            MarkSource::Proxy => "proxy",
        })
    }
}

/// Per-book proxy-mark policy: uncleared lookalike id → the cleared twin
/// whose settle serves as its official mark. Empty for almost every book;
/// populating it is an explicit valuation-policy decision, committed in
/// the book's config.
pub type ProxyMarks = std::collections::BTreeMap<String, String>;

/// A net position per instrument, derived from deals.
///
/// This is a derived view, not a trade event — unlike a `Deal` it has no
/// id, timestamp, or counterparty.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Position {
    pub instrument_id: String,
    /// Direction of the net position. A fully offset position is `Buy`
    /// with zero quantity.
    pub direction: BuySell,
    /// Net quantity, always non-negative (direction carries the sign).
    pub quantity: Decimal,
    /// Net entry price: |net cost / net quantity|. NOTE: this embeds the
    /// realized P&L of closed lots (buy 10 @ 71.80, sell 4 @ 72.50 →
    /// 71.33), it is NOT the FIFO cost basis of the remaining lots.
    /// Zero for fully offset positions.
    pub avg_price: Decimal,
}

/// Compress deals into net positions per instrument.
///
/// Fully offset positions (net qty = 0) are included with a zero price.
/// Result is sorted by instrument id.
pub fn compress(deals: &[Deal]) -> Vec<Position> {
    let mut groups: HashMap<String, (Decimal, Decimal)> = HashMap::new(); // (net_signed_qty, cost)

    for deal in deals {
        let entry = groups.entry(deal.instrument_id.clone()).or_default();
        let signed_qty = deal.signed_quantity();
        entry.0 += signed_qty;
        entry.1 += signed_qty * deal.price; // signed cost
    }

    let mut positions: Vec<Position> = groups
        .into_iter()
        .map(|(instrument_id, (net_qty, cost))| {
            let (direction, quantity) = if net_qty >= Decimal::ZERO {
                (BuySell::Buy, net_qty)
            } else {
                (BuySell::Sell, -net_qty)
            };

            let avg_price = if quantity != Decimal::ZERO {
                (cost / net_qty).abs()
            } else {
                Decimal::ZERO
            };

            Position {
                instrument_id,
                direction,
                quantity,
                avg_price,
            }
        })
        .collect();

    positions.sort_by(|a, b| a.instrument_id.cmp(&b.instrument_id));
    positions
}

/// Mark a set of deals against the portfolio's market data, producing one
/// valued deal per input deal. Deals that cannot be priced carry the error.
pub fn valuate(deals: &[Deal], portfolio: &Portfolio) -> Vec<ValuedDeal> {
    match portfolio.market_data.as_ref() {
        Some(md) => valuate_at(deals, &portfolio.instruments, md, &portfolio.proxy_marks),
        None => deals
            .iter()
            .map(|deal| ValuedDeal {
                deal: deal.clone(),
                valuation: Err(core::Error::Pricer(
                    "no market data loaded (valuation requires market_data)".to_string(),
                )),
            })
            .collect(),
    }
}

/// Mark a set of deals against an explicit market snapshot — the series
/// entry point: one call per trading day with that day's `MarketData`.
pub fn valuate_at(
    deals: &[Deal],
    instruments: &HashMap<String, Arc<dyn FinancialInstrument>>,
    md: &crate::market_data::MarketData,
    proxy_marks: &ProxyMarks,
) -> Vec<ValuedDeal> {
    deals
        .iter()
        .map(|deal| ValuedDeal {
            deal: deal.clone(),
            valuation: value_deal(deal, instruments, md, proxy_marks),
        })
        .collect()
}

fn value_deal(
    deal: &Deal,
    instruments: &HashMap<String, Arc<dyn FinancialInstrument>>,
    md: &crate::market_data::MarketData,
    proxy_marks: &ProxyMarks,
) -> core::Result<Valuation> {
    let inst = instruments.get(&deal.instrument_id).ok_or_else(|| {
        core::Error::Pricer(format!("unknown instrument '{}'", deal.instrument_id))
    })?;

    let (mark_f64, source) = official_mark(inst.as_ref(), md, proxy_marks)?;
    let mark = Decimal::try_from(mark_f64).map_err(|e| {
        core::Error::Pricer(format!(
            "cannot convert mark {} to decimal: {}",
            mark_f64, e
        ))
    })?;

    let pnl =
        deal.signed_quantity() * (mark - deal.price) * contract_size(inst.as_ref(), instruments);

    let realized = inst.maturity().is_some_and(|m| md.valuation_date() > m);

    Ok(Valuation {
        mark,
        pnl,
        realized,
        source,
    })
}

/// Official-valuation mark policy, dispatched on classification
/// (architecture.md §5):
///
/// - **cleared** → the own-venue settlement price; a missing settle is an
///   *error*, never a silent model fallback (a data failure must be loud);
/// - **uncleared** with a configured proxy → the cleared twin's settle
///   ([`MarkSource::Proxy`]); a missing twin settle is likewise an error;
/// - **uncleared** otherwise, and instrument types with no clearing
///   dimension → the model (which for expired instruments resolves to
///   settles/intrinsic).
fn official_mark(
    inst: &dyn FinancialInstrument,
    md: &crate::market_data::MarketData,
    proxy_marks: &ProxyMarks,
) -> core::Result<(f64, MarkSource)> {
    use crate::instruments::ClearingStatus;
    match inst.clearing() {
        Some(ClearingStatus::Cleared) => md
            .settlement_price(inst.id())
            .map(|p| (p, MarkSource::Settle))
            .map_err(|_| {
                core::Error::Pricer(format!(
                    "cleared instrument '{}': no settlement price at valuation date",
                    inst.id(),
                ))
            }),
        Some(ClearingStatus::Uncleared) if proxy_marks.contains_key(inst.id()) => {
            let twin = &proxy_marks[inst.id()];
            md.settlement_price(twin)
                .map(|p| (p, MarkSource::Proxy))
                .map_err(|_| {
                    core::Error::Pricer(format!(
                        "uncleared instrument '{}' proxies '{}': no settlement price for the twin at valuation date",
                        inst.id(),
                        twin,
                    ))
                })
        }
        _ => pricing::price(inst, md).map(|p| (p, MarkSource::Model)),
    }
}

/// Contract size for dollar-terms P&L: futures carry their own; options
/// inherit the underlying future's (a 10-lot option on 1000-bbl contracts
/// moving $0.50/bbl is $5,000). Everything else defaults to 1.
pub(crate) fn contract_size(
    inst: &dyn FinancialInstrument,
    instruments: &HashMap<String, Arc<dyn FinancialInstrument>>,
) -> Decimal {
    let any = inst.as_any();
    any.downcast_ref::<Future>()
        .map(|f| f.contract_size)
        .or_else(|| {
            any.downcast_ref::<EuropeanOption>()
                .and_then(|o| instruments.get(&o.underlying))
                .and_then(|u| u.as_any().downcast_ref::<Future>())
                .map(|f| f.contract_size)
        })
        .unwrap_or(Decimal::ONE)
}

/// Compute P&L totals from valued deals: (realized, unrealized).
/// Unpriced deals contribute nothing — callers should report them.
pub fn pnl_totals(valued: &[ValuedDeal]) -> (Decimal, Decimal) {
    let mut realized = Decimal::ZERO;
    let mut unrealized = Decimal::ZERO;
    for v in valued {
        if let Ok(val) = &v.valuation {
            if val.realized {
                realized += val.pnl;
            } else {
                unrealized += val.pnl;
            }
        }
    }
    (realized, unrealized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dates::Timestamp;

    fn make_deal(id: &str, instrument: &str, dir: BuySell, qty: u32, price: &str) -> Deal {
        Deal {
            id: id.to_string(),
            instrument_id: instrument.to_string(),
            direction: dir,
            quantity: Decimal::from(qty),
            price: price.parse().unwrap(),
            timestamp: Timestamp::parse("2026-03-02T10:00:00Z").unwrap(),
            counterparty: "TEST".to_string(),
        }
    }

    #[test]
    fn compress_nets_same_instrument() {
        let deals = vec![
            make_deal("D1", "ICE-BRN-K26", BuySell::Buy, 10, "71.80"),
            make_deal("D2", "ICE-BRN-K26", BuySell::Sell, 4, "72.50"),
        ];

        let positions = compress(&deals);
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].instrument_id, "ICE-BRN-K26");
        assert_eq!(positions[0].direction, BuySell::Buy);
        assert_eq!(positions[0].quantity, Decimal::from(6));
        // VWAP: (10*71.80 - 4*72.50) / 6 = (718 - 290) / 6 = 71.333...
        let expected_vwap: Decimal = "71.3333333333333333333333333333".parse().unwrap();
        assert_eq!(positions[0].avg_price, expected_vwap);
    }

    #[test]
    fn compress_fully_offset() {
        let deals = vec![
            make_deal("D1", "ICE-BRN-K26", BuySell::Buy, 10, "71.80"),
            make_deal("D2", "ICE-BRN-K26", BuySell::Sell, 10, "72.50"),
        ];

        let positions = compress(&deals);
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].quantity, Decimal::ZERO);
    }

    #[test]
    fn compress_multiple_instruments() {
        let deals = vec![
            make_deal("D1", "ICE-BRN-K26", BuySell::Buy, 10, "71.80"),
            make_deal("D2", "ICE-BRN-M26", BuySell::Sell, 5, "71.50"),
            make_deal("D3", "ICE-BRN-K26", BuySell::Buy, 5, "72.00"),
        ];

        let positions = compress(&deals);
        assert_eq!(positions.len(), 2);
        // Sorted by instrument_id
        assert_eq!(positions[0].instrument_id, "ICE-BRN-K26");
        assert_eq!(positions[0].quantity, Decimal::from(15));
        assert_eq!(positions[1].instrument_id, "ICE-BRN-M26");
        assert_eq!(positions[1].quantity, Decimal::from(5));
    }

    #[test]
    fn valuate_is_total_over_input_deals() {
        use crate::dates::Date;
        use crate::dates::daycount::DayCount;
        use crate::dates::rules::DateRule;
        use crate::instruments::{ClearingStatus, FinancialInstrument, Settlement};
        use crate::market_data::MarketData;
        use crate::reference_data::Currency;
        use std::sync::Arc;

        let usd = Arc::new(Currency::new("USD", DateRule::Null, DayCount::Act360));
        let settle = Settlement::new("ICE", "SETTLE", "19:30", "Europe/London", DateRule::Null);
        let priced = crate::instruments::Future::new(
            "PRICED",
            "Brent",
            usd.clone(),
            settle.clone(),
            ClearingStatus::Cleared,
            Date::new(2026, 6, 30),
            Decimal::from(1000),
            "0.01".parse().unwrap(),
        );
        let unpriced = crate::instruments::Future::new(
            "UNPRICED",
            "Brent",
            usd,
            settle,
            ClearingStatus::Cleared,
            Date::new(2026, 6, 30),
            Decimal::from(1000),
            "0.01".parse().unwrap(),
        );

        let valuation_date = Date::new(2026, 3, 7);
        let mut md = MarketData::new(valuation_date, valuation_date.as_of_midnight());
        // Settle-primary: cleared instruments mark from official settles.
        md.add_settlement_price("PRICED", 72.50); // no settle for UNPRICED

        let mut instruments: HashMap<String, Arc<dyn FinancialInstrument>> = HashMap::new();
        instruments.insert("PRICED".to_string(), Arc::new(priced));
        instruments.insert("UNPRICED".to_string(), Arc::new(unpriced));

        let deals = vec![
            make_deal("D1", "PRICED", BuySell::Buy, 10, "71.80"),
            make_deal("D2", "UNPRICED", BuySell::Buy, 5, "70.00"),
        ];
        let portfolio = Portfolio {
            instruments,
            deals: deals.clone(),
            market_data: Some(md),
            history: None,
            proxy_marks: Default::default(),
            indices: Default::default(),
            integrity_errors: vec![],
            warnings: vec![],
            reports: vec![],
        };

        let valued = valuate(&deals, &portfolio);
        // Every input deal appears in the output — failures are not dropped
        assert_eq!(valued.len(), 2);

        let v1 = valued.iter().find(|v| v.deal.id == "D1").unwrap();
        let val = v1.valuation.as_ref().unwrap();
        assert_eq!(val.mark, "72.50".parse::<Decimal>().unwrap());
        assert_eq!(val.source, MarkSource::Settle);
        // 10 * (72.50 - 71.80) * 1000 = 7000
        assert_eq!(val.pnl, Decimal::from(7000));

        // A cleared instrument with no settle errors — never a silent
        // model fallback.
        let v2 = valued.iter().find(|v| v.deal.id == "D2").unwrap();
        let err = v2.valuation.as_ref().unwrap_err().to_string();
        assert!(
            err.contains("no settlement price"),
            "expected missing-settlement error, got: {err}"
        );

        // Unpriced deals contribute nothing to totals
        let (realized, unrealized) = pnl_totals(&valued);
        assert_eq!(realized, Decimal::ZERO);
        assert_eq!(unrealized, Decimal::from(7000));
    }

    #[test]
    fn option_pnl_uses_underlying_contract_size() {
        use crate::curves::DiscountCurve;
        use crate::dates::Date;
        use crate::dates::daycount::DayCount;
        use crate::dates::rules::DateRule;
        use crate::instruments::{ClearingStatus, OptionSettlement, PutOrCall, Settlement};
        use crate::market_data::{MarketData, VolSurface};
        use crate::reference_data::Currency;
        use std::sync::Arc;

        let usd = Arc::new(Currency::new("USD", DateRule::Null, DayCount::Act360));
        let settle = Settlement::new("ICE", "SETTLE", "19:30", "Europe/London", DateRule::Null);
        let future = crate::instruments::Future::new(
            "FUT",
            "Brent",
            usd.clone(),
            settle.clone(),
            ClearingStatus::Cleared,
            Date::new(2026, 6, 30),
            Decimal::from(1000),
            "0.01".parse().unwrap(),
        );
        // Uncleared → the official mark is the model price (the policy's
        // model branch), which also exercises contract-size inheritance.
        let option = crate::instruments::EuropeanOption::new(
            "OPT",
            "FUT",
            "ICE",
            usd,
            settle,
            ClearingStatus::Uncleared,
            Date::new(2026, 6, 25),
            Decimal::from(75),
            PutOrCall::Call,
            OptionSettlement::Cash,
        );

        let valuation_date = Date::new(2026, 3, 10);
        let mut md = MarketData::new(valuation_date, valuation_date.as_of_midnight());
        md.add_market_price("FUT", 72.45);
        md.add_discount_curve(
            "USD",
            DiscountCurve::flat(valuation_date, DayCount::Act360, 0.04),
        );
        md.add_vol_surface("FUT", VolSurface::Flat { vol: 0.30 });

        let mut instruments: HashMap<String, Arc<dyn FinancialInstrument>> = HashMap::new();
        instruments.insert("FUT".to_string(), Arc::new(future));
        instruments.insert("OPT".to_string(), Arc::new(option));

        let deals = vec![make_deal("D1", "OPT", BuySell::Buy, 10, "2.50")];
        let portfolio = Portfolio {
            instruments,
            deals: deals.clone(),
            market_data: Some(md),
            history: None,
            proxy_marks: Default::default(),
            indices: Default::default(),
            integrity_errors: vec![],
            warnings: vec![],
            reports: vec![],
        };

        let valued = valuate(&deals, &portfolio);
        let val = valued[0].valuation.as_ref().unwrap();
        assert_eq!(val.source, MarkSource::Model);
        // P&L is in dollar terms: signed_qty * (mark - trade) * 1000 (the
        // underlying future's contract size, not the default of 1)
        let expected = Decimal::from(10)
            * (val.mark - "2.50".parse::<Decimal>().unwrap())
            * Decimal::from(1000);
        assert_eq!(val.pnl, expected);
        assert!(val.mark > Decimal::ZERO);
    }

    #[test]
    fn compress_net_short() {
        let deals = vec![
            make_deal("D1", "ICE-BRN-K26", BuySell::Sell, 10, "72.00"),
            make_deal("D2", "ICE-BRN-K26", BuySell::Buy, 3, "71.50"),
        ];

        let positions = compress(&deals);
        assert_eq!(positions[0].direction, BuySell::Sell);
        assert_eq!(positions[0].quantity, Decimal::from(7));
    }
}

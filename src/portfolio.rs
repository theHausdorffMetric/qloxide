use std::collections::HashMap;

use crate::Decimal;
use crate::config::Portfolio;
use crate::instruments::future::Future;
use crate::pricing;
use crate::trades::{BuySell, Deal};

/// A deal enriched with mark-to-market valuation.
pub struct ValuedDeal {
    pub deal: Deal,
    pub mark: Decimal,
    pub pnl: Decimal,
    pub realized: bool,
}

/// Compress deals into net positions per instrument (VWAP pricing).
///
/// Returns one synthetic deal per instrument with net quantity and
/// volume-weighted average price. Fully offset positions (net qty = 0)
/// are included with a zero price.
pub fn compress(deals: &[Deal]) -> Vec<Deal> {
    let mut groups: HashMap<String, (Decimal, Decimal)> = HashMap::new(); // (net_signed_qty, cost)

    for deal in deals {
        let entry = groups.entry(deal.instrument_id.clone()).or_default();
        let signed_qty = deal.signed_quantity();
        entry.0 += signed_qty;
        entry.1 += signed_qty * deal.price; // signed cost
    }

    let mut positions: Vec<Deal> = groups
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

            Deal {
                id: format!("NET-{}", instrument_id),
                instrument_id,
                direction,
                quantity,
                price: avg_price,
                timestamp: deals.last().unwrap().timestamp,
                counterparty: String::new(),
            }
        })
        .collect();

    positions.sort_by(|a, b| a.instrument_id.cmp(&b.instrument_id));
    positions
}

/// Mark a set of deals against market data, producing valued deals.
pub fn valuate(deals: &[Deal], portfolio: &Portfolio) -> Vec<ValuedDeal> {
    let md = &portfolio.market_data;

    deals
        .iter()
        .filter_map(|deal| {
            let inst = portfolio.instruments.get(&deal.instrument_id)?;

            let mark_f64 = pricing::price(inst.as_ref(), md).ok()?;
            let mark = Decimal::try_from(mark_f64).ok()?;

            let contract_size = inst
                .as_any()
                .downcast_ref::<Future>()
                .map(|f| f.contract_size)
                .unwrap_or(Decimal::ONE);

            let pnl = deal.signed_quantity() * (mark - deal.price) * contract_size;

            let realized = inst.maturity()
                .is_some_and(|m| md.spot_date() > m);

            Some(ValuedDeal {
                deal: deal.clone(),
                mark,
                pnl,
                realized,
            })
        })
        .collect()
}

/// Compute P&L totals from valued deals.
pub fn pnl_totals(valued: &[ValuedDeal]) -> (Decimal, Decimal) {
    let mut realized = Decimal::ZERO;
    let mut unrealized = Decimal::ZERO;
    for v in valued {
        if v.realized {
            realized += v.pnl;
        } else {
            unrealized += v.pnl;
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
        assert_eq!(positions[0].price, expected_vwap);
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

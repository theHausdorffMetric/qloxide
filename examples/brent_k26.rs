use std::collections::HashMap;
use std::sync::Arc;

use qloxide::Decimal;
use qloxide::instruments::FinancialInstrument;
use qloxide::instruments::future::Future;
use qloxide::market_data::MarketData;
use qloxide::pricing;
use qloxide::trades::Deal;

fn main() {
    // Load instrument
    let inst_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "examples/brent_k26.json".to_string());
    let inst_json = std::fs::read_to_string(&inst_path).unwrap_or_else(|e| {
        eprintln!("Failed to read {inst_path}: {e}");
        std::process::exit(1);
    });
    let inst: Arc<dyn FinancialInstrument> = serde_json::from_str(&inst_json).unwrap_or_else(|e| {
        eprintln!("Failed to parse instrument: {e}");
        std::process::exit(1);
    });

    // Build instrument registry
    let mut registry: HashMap<String, Arc<dyn FinancialInstrument>> = HashMap::new();
    registry.insert(inst.id().to_string(), inst);

    // Load deal
    let deal_path = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "examples/brent_k26_deal.json".to_string());
    let deal_json = std::fs::read_to_string(&deal_path).unwrap_or_else(|e| {
        eprintln!("Failed to read {deal_path}: {e}");
        std::process::exit(1);
    });
    let deal: Deal = serde_json::from_str(&deal_json).unwrap_or_else(|e| {
        eprintln!("Failed to parse deal: {e}");
        std::process::exit(1);
    });

    // Load market data
    let market_path = std::env::args()
        .nth(3)
        .unwrap_or_else(|| "examples/brent_k26_market.json".to_string());
    let market_json = std::fs::read_to_string(&market_path).unwrap_or_else(|e| {
        eprintln!("Failed to read {market_path}: {e}");
        std::process::exit(1);
    });
    let market: MarketData = serde_json::from_str(&market_json).unwrap_or_else(|e| {
        eprintln!("Failed to parse market data: {e}");
        std::process::exit(1);
    });

    // Resolve instrument
    let inst = registry.get(&deal.instrument_id).unwrap_or_else(|| {
        eprintln!("Unknown instrument: {}", deal.instrument_id);
        std::process::exit(1);
    });

    // Price via pricing module
    let mark_price = pricing::price(inst.as_ref(), &market).unwrap_or_else(|e| {
        eprintln!("Pricing failed: {e}");
        std::process::exit(1);
    });
    let mark_price = Decimal::try_from(mark_price).unwrap();

    // Downcast to Future to get contract_size
    let future: &Future = inst
        .as_ref()
        .as_any()
        .downcast_ref::<Future>()
        .unwrap_or_else(|| {
            eprintln!("Instrument {} is not a Future", deal.instrument_id);
            std::process::exit(1);
        });

    // Compute mark-to-market P&L
    let pnl = deal.signed_quantity() * (mark_price - deal.price) * future.contract_size;

    println!("=== Deal ===");
    println!("{deal}");
    println!();
    println!("=== Instrument ===");
    println!("ID:           {}", future.id);
    println!("Underlying:   {}", future.underlying);
    println!("Contract:     {} bbl", future.contract_size);
    println!("Expiry:       {}", future.expiry);
    println!();
    println!("=== Market Data ===");
    println!("Spot date:    {}", market.spot_date());
    println!("Mark price:   ${}/bbl", mark_price);
    println!();
    println!("=== Mark-to-Market ===");
    println!("P&L:          ${}", pnl);
}

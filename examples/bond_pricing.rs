use std::sync::Arc;

use qloxide::instruments::FinancialInstrument;
use qloxide::market_data::MarketData;
use qloxide::pricing;

fn main() {
    // Load instrument from JSON
    let inst_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "examples/bond_pricing.json".to_string());
    let inst_json = std::fs::read_to_string(&inst_path).unwrap_or_else(|e| {
        eprintln!("Failed to read {inst_path}: {e}");
        std::process::exit(1);
    });
    let inst: Arc<dyn FinancialInstrument> = serde_json::from_str(&inst_json).unwrap_or_else(|e| {
        eprintln!("Failed to parse instrument: {e}");
        std::process::exit(1);
    });

    // Load market data from JSON
    let market_path = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "examples/bond_pricing_market.json".to_string());
    let market_json = std::fs::read_to_string(&market_path).unwrap_or_else(|e| {
        eprintln!("Failed to read {market_path}: {e}");
        std::process::exit(1);
    });
    let market: MarketData = serde_json::from_str(&market_json).unwrap_or_else(|e| {
        eprintln!("Failed to parse market data: {e}");
        std::process::exit(1);
    });

    // Price the instrument
    let pv = pricing::price(inst.as_ref(), &market).unwrap_or_else(|e| {
        eprintln!("Pricing failed: {e}");
        std::process::exit(1);
    });

    println!("=== Instrument ===");
    println!("ID:         {}", inst.id());
    println!("Type:       {}", inst.instrument_type());
    println!("Currency:   {}", inst.currency().id);
    if let Some(mat) = inst.maturity() {
        println!("Maturity:   {}", mat);
    }
    println!();
    println!("=== Market Data ===");
    println!("Spot date:  {}", market.spot_date());
    println!();
    println!("=== Pricing ===");
    println!("PV:         {:.4}", pv);
    println!("PV (per 100 face): {:.4}", pv);
}

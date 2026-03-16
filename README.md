# qloxide

A Rust library for financial instrument modeling, trade management, and pricing. Targeting energy, refined products, and commodities — but designed generally.

**Status: active development.** Layers 0–5 are complete with comprehensive tests. Deterministic pricing (bonds, futures) works. Option/swap pricing, vol surfaces, Greeks, and forward curves are not yet implemented.

## Architecture

Strict layered hierarchy — each layer depends only on layers below it:

| Layer | Contents | Status |
|-------|----------|--------|
| 0 — Foundation | `Error`, `Date`, `Timestamp`, `Zoned` (jiff newtypes) | Done |
| 1 — Date Logic | `Calendar`, `DateRule`, `DayCount` (5 conventions), `Compounding` (6 conventions) | Done |
| 2 — Reference Data | `Currency`, `CreditEntity`, `RateIndex` | Done |
| 3 — Cash Flows | `CashFlow`, `CashFlowSchedule`, `Frequency` | Done |
| 4 — Curves | `DiscountCurve` (parameterized by DayCount, linear interpolation on r*t) | Done |
| 5 — Instruments | `FinancialInstrument` trait + 7 types: Equity, Future, EuropeanOption, Bond, Swap, FxForward, Basket | Done |
| 5.5 — Trades | `Deal`, `BuySell`, portfolio compression (VWAP), P&L | Done |
| 6 — Forward Curves | Commodity forward term structure | Not started |
| 7 — Pricing | `PricingContext` trait, Bond pricer, Future pricer | Partial |
| 8 — Risk | Greeks, scenarios, bump-and-reprice | Not started |

## Modules

```
src/
├── lib.rs              # public API
├── main.rs             # CLI (qloxide --config pricing.toml)
├── core/               # Error enum (thiserror)
├── dates/              # Date, Timestamp, Zoned, Calendar, DateRule, DayCount, Compounding
├── reference_data/     # Currency, CreditEntity, RateIndex
├── cashflows/          # CashFlow, CashFlowSchedule, Frequency
├── curves/             # DiscountCurve
├── instruments/        # FinancialInstrument trait + 7 implementations
├── market_data/        # MarketData: spots, discount curves, settlement prices
├── pricing/            # PricingContext trait, bond + future pricers
├── trades/             # Deal, BuySell enum
├── config.rs           # TOML config loader with validation
├── portfolio.rs        # Position compression, mark-to-market, P&L
└── reports.rs          # Text reports: instruments, deals, positions, pnl
```

## Design choices

- **Trait-based instruments** with `typetag` — add new instrument types without touching existing code. `Arc<dyn FinancialInstrument>` serializes to/from JSON.
- **Pricing is a separate module** — instruments are pure data. Three independent inputs: instrument, market data (`PricingContext`), and model (for non-deterministic instruments).
- **Currency is reference data**, not an instrument — cleaner separation for trade management.
- **Cash flows are first-class** — every instrument ultimately produces cash flows.
- **Three time types** — `Date` (calendar), `Timestamp` (UTC instant), `Zoned` (datetime + timezone) — because energy settlement needs all three.
- **Parameterized day counts** — curves aren't hardcoded to Act/365.
- **`rust_decimal::Decimal`** for prices, quantities, and rates — no floating-point rounding.
- **Settlement struct** captures venue, session, time, timezone, and payment lag — not just a bare date rule.

## CLI

```bash
qloxide --config pricing.toml
qloxide --config pricing.toml --report pnl --report instruments
```

The TOML config references JSON files for instruments, deals, and market data. Available reports: `instruments`, `deals`, `positions`, `pnl`.

## Example

```rust
use std::sync::Arc;
use qloxide::dates::Date;
use qloxide::dates::daycount::DayCount;
use qloxide::dates::rules::DateRule;
use qloxide::instruments::*;
use qloxide::reference_data::Currency;

let usd = Arc::new(Currency::new("USD", DateRule::Null, DayCount::Act360));

let future = Future::new(
    "ICE-BRN-Jun25", "Brent", usd,
    Settlement::new("ICE", "SETTLE", "19:30", "Europe/London", DateRule::Null),
    Date::new(2025, 6, 14), 1000.0, 0.01,
);

// Serialize to JSON
let json = serde_json::to_string_pretty(
    &(Arc::new(future) as Arc<dyn FinancialInstrument>)
).unwrap();

// Deserialize back
let inst: Arc<dyn FinancialInstrument> = serde_json::from_str(&json).unwrap();
assert_eq!(inst.id(), "ICE-BRN-Jun25");
```

## Build

```bash
cargo build
cargo test
cargo run -- --config examples/brent/pricing.toml
```

## Dependencies

`jiff` (dates/timezones), `rust_decimal` (exact arithmetic), `serde` + `serde_json` + `typetag` (polymorphic serialization), `thiserror` (errors), `clap` (CLI), `toml` (config).

## License

MIT

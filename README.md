# qloxide

A Rust library for financial instrument modeling, trade management, and pricing. Targeting energy, refined products, and commodities — but designed generally.

**Status: active development.** Layers 0–5.5 are complete with comprehensive tests. Deterministic pricing (bonds, futures) works. Option/swap pricing, vol surfaces, Greeks, and forward curves are not yet implemented.

## Architecture

Design decisions, reasoning, and rejected alternatives are documented in
[ARCHITECTURE.md](ARCHITECTURE.md); further planning, review, and reference
notes (including the upstream [QuantMath](https://github.com/MarcusRainbow/QuantMath.git)
it reimplements) live in [`docs/`](docs/). The short version: strict layered
hierarchy — each layer depends only on layers below it:

| Layer | Contents | Status |
|-------|----------|--------|
| 0 — Foundation | `Error`, `Date`, `Time`, `Timestamp`, `Zoned` (jiff newtypes) | Done |
| 1 — Date Logic | `Calendar`, `DateRule`, `DayCount` (4 conventions), `Compounding` (6 conventions) | Done |
| 2 — Reference Data | `Currency`, `CreditEntity`, `RateIndex` | Done |
| 3 — Cash Flows | `CashFlow`, `CashFlowSchedule`, `Frequency` | Done |
| 4 — Curves | `DiscountCurve` (parameterized by DayCount, linear interpolation on r*t) | Done |
| 5 — Instruments | `FinancialInstrument` trait + 7 types: Equity, Future, EuropeanOption, Bond, Swap, FxForward, Basket | Done |
| 5.5 — Trades | `Deal`, `BuySell`, `Position` (VWAP compression), P&L | Done |
| 6 — Forward Curves | Commodity forward term structure | Not started |
| 7 — Pricing | `PricingContext` trait, Bond pricer, Future pricer | Partial |
| 8 — Risk | Greeks, scenarios, bump-and-reprice | Not started |

## Modules

```
src/
├── lib.rs              # public API
├── bin/                # qloxide-book + qloxide-risk (thin wrappers over cli.rs)
├── cli.rs              # shared CLI driver, report tiers
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
- **Pricing is a separate module** — instruments are pure data. One uniform interface: `price(instrument, context)`; the model choice (Black76 vs Bachelier, when options land) is implicit in the vol surface variant.
- **Currency is reference data**, not an instrument — cleaner separation for trade management.
- **Cash flows are first-class** — instruments decompose into contractual cash flows (`Bond::cash_flows()`), with amounts computed exactly in `Decimal`.
- **Four time types** — `Date` (calendar), `Time` (session time of day), `Timestamp` (UTC instant), `Zoned` (datetime + timezone) — because energy settlement needs them all.
- **Parameterized day counts** — curves aren't hardcoded to Act/365.
- **`rust_decimal::Decimal`** for prices, quantities, and rates — no floating-point rounding.
- **Settlement struct** captures venue, session, time, timezone, and payment lag — not just a bare date rule.

## CLI

The executable surface is split by trust tier: `qloxide-book` serves the
official world and structurally cannot emit model numbers; `qloxide-risk`
serves the model world, including `--market` scenario overlays.

```bash
qloxide-book --config pricing.toml
qloxide-book --config pricing.toml --report pnl --report instruments
qloxide-risk --config pricing.toml                    # risk report
qloxide-risk --config pricing.toml --market bump.json # scenario overlay
```

The TOML config references JSON files for instruments, deals, and market data. Book reports: `instruments`, `deals`, `positions`, `pnl`, `pnl-series`. Risk reports: `risk`. Both binaries share the same config; each runs only its own tier's reports. Book market data is vol-free (surfaces appear only where an uncleared position needs them for marking); the optional `vol_data` key lists risk-tier surface files that `qloxide-risk` merges and `qloxide-book` ignores.

## Example

```rust
use std::sync::Arc;
use qloxide::Decimal;
use qloxide::dates::Date;
use qloxide::dates::daycount::DayCount;
use qloxide::dates::rules::DateRule;
use qloxide::instruments::*;
use qloxide::reference_data::Currency;

let usd = Arc::new(Currency::new("USD", DateRule::Null, DayCount::Act360));

let future = Future::new(
    "ICE-BRN-Jun25", "Brent", usd,
    Settlement::new("ICE", "SETTLE", "19:30", "Europe/London", DateRule::Null),
    Date::new(2025, 6, 14),
    Decimal::from(1000),        // contract size (bbl)
    "0.01".parse().unwrap(),    // tick size
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
cargo run --bin qloxide-book -- --config examples/brent/brent.toml
```

## Dependencies

`jiff` (dates/timezones), `rust_decimal` (exact arithmetic), `serde` + `serde_json` + `typetag` (polymorphic serialization), `thiserror` (errors), `toml` (config), `clap` (CLI — optional; disable with `default-features = false`).

## License

GPL-3.0-or-later, from 0.3.0 onward. Versions up to and including 0.2.0 were
published under MIT; that grant remains valid for those versions.

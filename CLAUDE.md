# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

qloxide is a Rust library for financial instrument modeling, trade management, and pricing. Target domains: energy, refined products, commodities. Author: Daniel Probst. License: MIT.

Published on crates.io as `qloxide`. Hosted on sr.ht: `git.sr.ht/~danprobst/qloxide`.

## Build & Test Commands

```bash
cargo build                       # Build library
cargo test --all                  # Run all tests (74 tests)
cargo test <test_name>            # Run a single test by name
cargo test -- --nocapture         # Run tests with stdout visible
cargo run --example brent_k26    # Run the Brent K26 example
cargo clippy                      # Lint
rustfmt src/**/*.rs               # Format
```

## Architecture

Strict layered module hierarchy — each layer depends only on layers below it:

```
Layer 0: core           — Error enum (thiserror), Result alias
Layer 1: dates          — Date/Timestamp/Zoned newtypes (jiff), Calendar,
                          DateRule, DayCount, Compounding
Layer 2: reference_data — Currency, CreditEntity, RateIndex
Layer 3: cashflows      — CashFlow (Decimal amount), CashFlowSchedule, Frequency
Layer 4: curves         — DiscountCurve (f64, parameterized by DayCount)
Layer 5: instruments    — FinancialInstrument trait (typetag), 7 types:
                          Bond, Equity, EuropeanOption, Future, FxForward, Swap, Basket
Layer 5.5: trades       — Deal, BuySell (instrument_id reference, Decimal for price/qty)
```

**Not yet implemented:**
- Layer 6: Forward curves (commodity forward term structure from futures prices)
- Layer 7: Pricing & risk (valuation, Greeks, scenarios)

## Key Design Patterns

- **typetag for polymorphism** — `#[typetag::serde(tag = "type")]` on `FinancialInstrument` trait enables open extensibility and automatic tagged JSON serialization. New instrument types require only a new file + `#[typetag::serde] impl`.
- **Decimal for financial amounts, f64 for curves** — `rust_decimal::Decimal` for cash flow amounts, trade prices, and quantities (exact representation). `f64` for discount curves, year fractions, and interpolation (performance).
- **Currency as reference data** — not an instrument. Instruments are denominated in a currency via `Arc<Currency>`.
- **Three time types** — `Date` (calendar math, curve pillars), `Timestamp` (trade execution, UTC), `Zoned` (settlement sessions, timezone-aware).
- **DayCount-parameterized curves** — `DiscountCurve` takes a `DayCount` enum, not hardcoded Act/365.
- **Settlement struct** — venue + session + time + timezone, replacing QuantMath's bare `TimeOfDay` enum.
- **`as_any()` for downcasting** — `FinancialInstrument` trait includes `as_any() -> &dyn Any` for concrete type access when needed.

## Module Structure

```
src/
  lib.rs                    pub mod + re-export Decimal
  core/
    mod.rs                  Error re-export, Result<T> alias
    error.rs                Error enum (thiserror)
  dates/
    mod.rs                  Date, Timestamp, Zoned newtypes (jiff)
    calendar.rs             Calendar enum (EveryDay, Weekday, WeekdayAndHoliday)
    daycount.rs             DayCount enum, Compounding enum
    rules.rs                DateRule enum (Null, BusinessDays, ModifiedFollowing)
  reference_data/
    mod.rs                  Re-exports
    currency.rs             Currency { id, settlement, day_count }
    credit_entity.rs        CreditEntity { id, currency }
    rate_index.rs           RateIndex { id, currency, tenor, ... }
  cashflows/
    mod.rs                  CashFlow, CashFlowSchedule, Frequency
  curves/
    mod.rs                  DiscountCurve (pillar interpolation, df, zero_rate, forward_rate)
  instruments/
    mod.rs                  FinancialInstrument trait, Settlement, PutOrCall, ExerciseStyle
    bond.rs                 Bond
    equity.rs               Equity
    option.rs               EuropeanOption
    future.rs               Future
    fx.rs                   FxForward
    swap.rs                 Swap (FixedLeg, FloatingLeg, PayReceive)
    basket.rs               Basket (recursive composition)
  trades/
    mod.rs                  Deal, BuySell
examples/
  brent_k26.rs              ICE Brent crude oil example
tests/
  instrument_json.rs        Integration tests for JSON serialization
```

## Dependencies

```toml
thiserror = "2"                                    # Error handling
serde = { version = "1", features = ["derive", "rc"] }  # Serialization
serde_json = "1"                                   # JSON
jiff = { version = "0.2", features = ["serde"] }   # Date/time
typetag = "0.2"                                    # Trait object serialization
rust_decimal = { version = "1", features = ["serde-with-str"] }  # Exact decimals
```

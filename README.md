# qloxide

A Rust library for financial instrument modeling, trade management, and pricing. Targeting energy, refined products, and commodities — but designed generally.

**Status: early development / incomplete.** The core type system is in place but pricing, risk, and trade management layers are not yet implemented. The API will change.

## What's here

Layers 0–5 of a planned 8-layer architecture:

| Layer | Contents | Status |
|-------|----------|--------|
| 0 — Foundation | `Error`, `Date`, `Timestamp`, `Zoned` (jiff newtypes) | Done |
| 1 — Date Logic | `Calendar`, `DateRule`, `DayCount` (5 conventions), `Compounding` (6 conventions) | Done |
| 2 — Reference Data | `Currency`, `CreditEntity`, `RateIndex` | Done |
| 3 — Cash Flows | `CashFlow`, `CashFlowSchedule`, `Frequency` | Done |
| 4 — Curves | `DiscountCurve` (parameterized by DayCount, linear interpolation) | Done |
| 5 — Instruments | `FinancialInstrument` trait with 7 types: Equity, Future, EuropeanOption, Bond, Swap, FxForward, Basket | Done |
| 6 — Forward Curves | Commodity forward term structure | Not started |
| 7 — Pricing & Risk | Valuation, Greeks, scenarios | Not started |

## Design choices

- **Trait-based instruments** with `typetag` — add new instrument types without touching existing code. `Arc<dyn FinancialInstrument>` serializes to/from JSON automatically.
- **Currency is reference data**, not an instrument — cleaner separation for trade management.
- **Cash flows are first-class** — every instrument ultimately produces cash flows.
- **Three time types** — `Date` (calendar), `Timestamp` (UTC instant), `Zoned` (datetime + timezone) — because energy settlement needs all three.
- **Parameterized day counts** — curves aren't hardcoded to Act/365.

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
    Settlement::new("ICE", "SETTLE", "19:30", "Europe/London"),
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

## License

MIT

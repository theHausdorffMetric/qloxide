# qloxide: Incremental Build Plan — Instruments & JSON Serialization

> **Status (2026-03-06):** All 12 original steps are complete or superseded. The architecture diverged from this plan in several ways: Currency moved to `reference_data` (not `instruments`), `typetag` trait replaced closed enum, `TimeOfDay`/`DateDayFraction`/`ZeroCoupon` were not implemented (replaced by different designs), and a trades layer (Deal, BuySell) was added beyond the original plan. See `qloxide-architecture.md` for the current design.

## Context

Build `qloxide` (`/home/dan/dev/ql/qloxide/`) from scratch, one type at a time, simplest to most complex. Each step adds a small cohesive group of types, compiles, tests JSON roundtrip, and pauses for discussion before proceeding. Reference implementation at `/home/dan/dev/ql/QuantMath-test-rs/`.

**Confirmed decisions:** jiff newtype for Date, `thiserror` errors, closed enums, serde derive, Rust 2024 edition. Currency ownership starts as String, discussed at Step 6.

---

## Steps

### Step 0: Skeleton + Error

**Add:** `Error` enum (all variants upfront — they're just string wrappers, avoids churn)

**Files:**
- `Cargo.toml` — add `thiserror = "2"`, `serde` (derive, rc), `serde_json`, `jiff = "0.2"`
- `src/lib.rs` — `pub mod core;`
- `src/core/mod.rs` + `src/core/error.rs`

**Tests:** error display, unwrap-panics

**Status:** Done

---

### Step 1: Date newtype (jiff)

**Add:** `Date(jiff::civil::Date)` with custom serde as `"YYYY-MM-DD"`

**Files:**
- `src/lib.rs` — add `pub mod dates;`
- `src/dates/mod.rs` — Date struct (inline or submod)

**Provides:** `new(y,m,d)`, `FromStr`, `Display`, `Add/Sub<i32>`, `Sub<Date>→i32`, `day_of_week()→u8` (0=Mon..6=Sun), `Ord`, custom Serialize/Deserialize

**Discuss:** panicking `new()` vs fallible `try_new()`, valid date range

**JSON:** `"2024-06-14"`

**Tests:** construction, from_str, arithmetic, day_of_week, serde roundtrip, ordering

**Status:** Done

---

### Step 2: TimeOfDay + DateTime + DateDayFraction

**Add:** 3 small time types

**File:** `src/dates/datetime.rs`

| Type | Fields | Serde |
|------|--------|-------|
| `TimeOfDay` | enum: Open, EDSP, Close | `"Close"` |
| `DateTime` | `{date, time_of_day}` | `{"date":"2024-06-14","time_of_day":"Close"}` |
| `DateDayFraction` | `{date, day_fraction}` | `{"date":"2024-06-14","day_fraction":0.8}` |

**Discuss:** `DateDayFraction::start()` sentinel — use `Option<DateDayFraction>` vs special nil-date value? Reference uses nil-date sentinel.

**Tests:** ordering, serde roundtrips, arithmetic

**Status:** Superseded — TimeOfDay, DateTime, DateDayFraction not implemented. Replaced by three jiff newtypes (Date, Timestamp, Zoned) and Settlement struct with venue/session/time/timezone.

---

### Step 3: Calendar enum

**Add:** `Calendar` — 4 variants (EveryDay, Weekday, WeekdayAndHoliday, Volatility)

**File:** `src/dates/calendar.rs` (largest single file — business day counting logic)

**Reference:** `QuantMath-test-rs/src/dates/calendar.rs` (661 lines with tests)

**Key methods:** `is_holiday()`, `count_business_days()`, `step()`, `year_fraction()`

**Discuss:** `Volatility` variant uses `Box<Calendar>` for recursive sizing

**JSON:** `"Weekday"` or `{"WeekdayAndHoliday":{"name":"LSE","holidays":["2024-12-25"]}}`

**Tests:** holiday checks, count consistency (brute-force vs algorithmic), step consistency

**Status:** Done — Calendar implemented with EveryDay, Weekday, WeekdayAndHoliday variants (Volatility variant not included). DayCount and Compounding enums also implemented in this layer.

---

### Step 4: DateRule enum

**Add:** `DateRule` — 3 variants (Null, BusinessDays, ModifiedFollowing)

**File:** `src/dates/rules.rs`

**Key method:** `apply(Date) → Date` — adjusts dates to business days

**JSON:** `"Null"` or `{"BusinessDays":{"calendar":"Weekday","step":2,"slip_forward":true}}`

**Tests:** next/prev business day, modified following at month boundary, serde roundtrip

**Status:** Done

---

### Step 5: Currency

**Add:** `Currency` struct — simplest instrument

**Files:**
- `src/lib.rs` — add `pub mod instruments;`
- `src/instruments/mod.rs` — `pub mod assets;`
- `src/instruments/assets.rs` — `Currency { id: String, settlement: DateRule }`

**JSON:** `{"id":"USD","settlement":"Null"}`

**Tests:** construction, serde roundtrip

**Status:** Done — redesigned. Currency moved to `reference_data` module (not `instruments`). Fields: `{ id, settlement: DateRule, day_count: DayCount }`. Currency is reference data, not an instrument.

---

### Step 6: CreditEntity + Equity

**Add:** two more asset types to `instruments/assets.rs`

**Fields:**
- `CreditEntity { id, currency: ?, settlement }`
- `Equity { id, credit_id, currency: ?, settlement }` + `time_to_day_fraction()` method

**Discuss: Currency ownership model.** Reference uses `Arc<Currency>`. Options:
- `Arc<Currency>` — self-contained JSON, `payoff_currency()` returns `&Currency`. Needs serde `rc` feature (already enabled).
- `String` — simpler, but later `payoff_currency()` would need a lookup table.
- **Recommendation:** `Arc<Currency>` since it's needed by ZeroCoupon decomposition and option exercise.

**JSON (with Arc):**
```json
{"id":"AAPL","credit_id":"OPT","currency":{"id":"USD","settlement":"Null"},"settlement":"Null"}
```

**Tests:** construction, time_to_day_fraction (Close->0.8, Open->0.0), serde roundtrip

**Status:** Done — redesigned. CreditEntity and RateIndex moved to `reference_data` module. Equity implemented as a `FinancialInstrument` trait impl with `typetag` (not an enum variant). Uses `Arc<Currency>` for currency ownership.

---

### Step 7: PutOrCall + OptionSettlement + VanillaOption

**Add:** option building blocks to `src/instruments/options.rs`

- `PutOrCall { Put, Call }` — serde as `"Call"` / `"Put"`
- `OptionSettlement { Cash, Physical }`
- `VanillaOption` — shared field bundle (not an instrument itself):
  `{ id, credit_id, underlying_id, currency: Arc<Currency>, settlement, expiry: DateTime, put_or_call, cash_or_physical, expiry_time: DateDayFraction, pay_date: Date }`

**Discuss:** `expiry_time` and `pay_date` are computed in `new()` but stored+serialized. If hand-edited JSON has inconsistent values, no validation. Acceptable? (Reference does it this way.)

**Tests:** pay_date computation, expiry_time = 0.8 for Close, serde roundtrip

**Status:** Done — redesigned. No VanillaOption bundle. Implemented as `EuropeanOption` with direct fields: strike, put_or_call, exercise_style, option_settlement, expiry, Settlement struct. Uses `typetag` trait impl.

---

### Step 8: SpotStartingEuropean + ForwardStartingEuropean

**Add:** two option types composing VanillaOption

- `SpotStartingEuropean { vanilla: VanillaOption, strike: f64 }`
- `ForwardStartingEuropean { vanilla: VanillaOption, strike_fraction: f64, strike_date: DateTime, strike_time: DateDayFraction }`

**JSON:**
```json
{"vanilla":{...},"strike":190.0}
```

**Tests:** construction, computed strike_time, serde roundtrip

**Status:** Superseded — no SpotStartingEuropean or ForwardStartingEuropean. Single `EuropeanOption` type covers the use case. Forward-starting options deferred to pricing layer.

---

### Step 9: ZeroCoupon

**Add:** `ZeroCoupon` to `src/instruments/bonds.rs`

`{ id, credit_id, currency: Arc<Currency>, ex_date: DateTime, payment_date: Date, settlement: DateRule }`

**Discuss:** `ex_date` (DateTime — when bond dies) vs `payment_date` (Date — when cash moves). These are different dates with different precision.

**Tests:** construction, serde roundtrip

**Status:** Superseded — no ZeroCoupon type. Replaced by `Bond` with coupon rate, maturity, face value. Cash flows modeled via `CashFlow` type with `Decimal` amounts.

---

### Step 10: Basket + Instrument enum

**Add:** `Basket` + `Instrument` enum together (co-dependent: Basket contains `Vec<(f64, Arc<Instrument>)>`)

**Files:**
- `src/instruments/basket.rs` — `Basket { id, credit_id, currency: Arc<Currency>, settlement, components: Vec<(f64, Arc<Instrument>)> }`
- `src/instruments/mod.rs` — `Instrument` enum (7 variants) with structural methods: `id()`, `payoff_currency()`, `credit_id()`, `settlement()`, `is_pure_rates()`

**JSON (recursive basket):**
```json
{"Basket":{"id":"B1","credit_id":"OPT","currency":{...},"settlement":"Null",
  "components":[[0.5,{"Equity":{...}}],[0.5,{"SpotStartingEuropean":{...}}]]}}
```

**Tests:** id/credit dispatch, serde roundtrip for each variant, nested basket roundtrip, `is_pure_rates()`

**Status:** Done — redesigned. No `Instrument` enum. `FinancialInstrument` is a `typetag` trait with 6 methods (id, currency, settlement, maturity, instrument_type, as_any). Basket holds `Vec<(f64, Arc<dyn FinancialInstrument>)>`. 7 implementing types: Bond, Equity, EuropeanOption, Future, FxForward, Swap, Basket.

---

### Step 11: Integration tests

**Add:** `tests/instrument_json.rs` — end-to-end JSON contract validation

- Construct realistic instruments, serialize, inspect JSON shape
- Deserialize from hand-written JSON strings
- Nested basket with options and bonds
- Malformed JSON → clear errors

**Status:** Done — 12 integration tests in `tests/instrument_json.rs` covering all 7 instrument types plus nested baskets.

---

## Final Module Structure

```
qloxide/src/
├── lib.rs                  pub mod core, dates, instruments
├── core/
│   ├── mod.rs              Error re-export, Result<T> alias
│   └── error.rs            Error enum (thiserror)
├── dates/
│   ├── mod.rs              Date newtype (jiff wrapper)
│   ├── datetime.rs         TimeOfDay, DateTime, DateDayFraction
│   ├── calendar.rs         Calendar enum (4 variants)
│   └── rules.rs            DateRule enum (3 variants)
└── instruments/
    ├── mod.rs              Instrument enum (7 variants) + accessor methods
    ├── assets.rs           Currency, CreditEntity, Equity
    ├── options.rs          PutOrCall, OptionSettlement, VanillaOption, SpotStarting, ForwardStarting
    ├── bonds.rs            ZeroCoupon
    └── basket.rs           Basket
```

## Cargo.toml

```toml
[package]
name = "qloxide"
version = "0.1.0"
edition = "2024"

[dependencies]
thiserror = "2"
serde = { version = "1", features = ["derive", "rc"] }
serde_json = "1"
jiff = "0.2"
```

## Type Dependency Graph

```
Error                    ← foundational, no deps
Date(jiff::civil::Date)  ← custom serde "YYYY-MM-DD"
TimeOfDay enum           ← no deps
DateTime                 ← Date + TimeOfDay
DateDayFraction          ← Date + f64
Calendar enum            ← Date, DateDayFraction
DateRule enum             ← Calendar, Date
Currency                 ← DateRule
CreditEntity             ← Currency ref, DateRule
Equity                   ← Currency ref, DateRule, DateTime, DateDayFraction
ZeroCoupon               ← Currency ref, DateTime, Date, DateRule
VanillaOption            ← Currency ref, DateTime, DateDayFraction, DateRule
SpotStartingEuropean     ← VanillaOption + f64 strike
ForwardStartingEuropean  ← VanillaOption + f64, DateTime, DateDayFraction
Basket                   ← Currency ref, DateRule, Vec<(f64, Arc<Instrument>)>
Instrument enum          ← all of the above
```

## Verification

After each step: `cargo build` + `cargo test` must pass. JSON output printed in tests (with `-- --nocapture`) for visual inspection during discussion.

## Design Decisions To Discuss Per Step

| Step | Decision |
|------|----------|
| 1 | Panicking `new()` vs fallible `try_new()` for Date |
| 2 | `DateDayFraction::start()` sentinel vs `Option<DateDayFraction>` |
| 3 | `Box<Calendar>` in Volatility variant |
| 6 | `Arc<Currency>` vs `String` for currency field in Equity/CreditEntity |
| 7 | Computed fields (expiry_time, pay_date) stored+serialized vs recomputed |
| 9 | ex_date (DateTime) vs payment_date (Date) — different precision, why |

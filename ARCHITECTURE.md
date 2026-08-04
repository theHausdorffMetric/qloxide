# qloxide: Architecture Decisions

Why qloxide is designed the way it is — decisions, reasoning, and the
alternatives that were rejected. For *what* the API does, see the rustdoc;
for release history, see CHANGELOG.md. When a decision is revised, the old
one stays with a supersession note rather than being silently rewritten.

## 1. Project Vision & Scope

### 1.1 What is qloxide?

A Rust library for financial instrument modeling, trade management, and pricing. Target domains: energy, refined products, commodities — but designed generally.

### 1.2 Intended use cases

| Use case | Description |
|----------|-------------|
| **Instrument definitions** | Define financial contracts (futures, swaps, options, bonds) with full JSON serialization |
| **Trade management** | Deal capture, trade blotter, position netting |
| **Reconciliation** | Match trades across systems (requires precise venue/settlement identity) |
| **Portfolio compression** | Identify offsetting positions for netting |
| **Pricing & risk** | Risk-neutral valuation, Greeks, scenario analysis (later layers) |

### 1.3 Why not use QuantMath directly?

> **QuantMath** (Marcus Rainbow, MIT): <https://github.com/MarcusRainbow/QuantMath.git>
> — the reference qloxide reimplements from scratch (pinned ref `b51ffac`). See
> [`docs/quantmath-upstream.md`](docs/quantmath-upstream.md) for notes on it.

QuantMath is a pricing-only library. It makes simplifications appropriate for equity derivatives but problematic for energy/commodities and trade management:

- `TimeOfDay { Open, EDSP, Close }` — too thin for energy markets where ICE Brent settles at 19:30 London, CME Brent on a different schedule, and Naphtha has distinct Singapore vs London closes
- `Currency` as an instrument variant — a modeling trick for pricing, not meaningful for trade management
- No cash flow type — options decompose directly to `ZeroCoupon` bonds with no intermediate representation
- Hardcoded Act/365 day count — energy swaps use Act/360, BRL uses Act/252
- No rate index definitions — floating legs need to know "3M SOFR" as a defined object
- No commodity forward curves — energy forwards come from futures prices, not spot + interest rates
- Closed enum for instruments — can't add new types without modifying existing code

### 1.4 Technical foundations

| Decision | Choice | Status |
|----------|--------|--------|
| Language edition | Rust 2024 | Confirmed |
| Error handling | `thiserror` | Confirmed |
| Serialization | `serde` with derive + `typetag` for trait objects | Confirmed |
| Date/time | `jiff` newtypes | Confirmed |
| Instrument polymorphism | Trait-based with `typetag` | Confirmed |
| Financial amounts | `rust_decimal::Decimal` | Confirmed |

---

## 2. Time Types

### 2.1 The problem

Financial systems need dates and times at different levels of precision:
- **Calendar date** — "14 June 2025" — for curve pillars, holidays, settlement dates
- **Execution timestamp** — "2025-06-14T14:32:07.123Z" — for trade blotters, audit trails
- **Zoned datetime** — "19:30 Europe/London on 14 June 2025" — for settlement sessions, observation times

QuantMath uses only `Date(i32)` (Truncated Julian integer) with a bare `TimeOfDay` enum. This conflates "which day" with "at what time and where," which breaks for energy/commodity markets.

### 2.2 Decision: Three jiff newtypes

| Type | Wraps | Purpose | Example |
|------|-------|---------|---------|
| `Date` | `jiff::civil::Date` | Calendar math, holidays, year fractions, curve pillars | 2025-06-14 |
| `Timestamp` | `jiff::Timestamp` | Trade execution time, event logging (absolute UTC instant) | 2025-06-14T14:32:07Z |
| `Zoned` | `jiff::Zoned` | Settlement times, observation times (datetime + timezone) | 2025-06-14T19:30 Europe/London |
| `Time` | `jiff::civil::Time` | Settlement session time of day, minute precision (added later) | 19:30 |

### 2.3 Reasoning

- **Civil `Date` for pricing math** — yield curves, vol surfaces, business day counting all operate on calendar days. Adding timezone context would complicate every lookup for no benefit. This is the same abstraction every quant library uses (QuantLib, OpenGamma, QuantMath).

- **`Timestamp` for operational events** — trade executions, system events, audit logs need absolute time (UTC). Not tied to any financial center.

- **`Zoned` for settlement/observation** — "ICE Brent settles at 19:30 London" is a zoned datetime. This is essential for reconciliation (matching trades requires matching settlement sessions) and for energy markets (Singapore close vs London close are different prices).

- **Why jiff?** — jiff provides all three types with good interop, correct timezone handling (IANA database), and Rust-native ergonomics. The alternative (chrono) has known soundness issues and less ergonomic timezone support.

### 2.4 Date construction: `new()` + `try_new()`

**Decision:** Provide both.
- `Date::new(2025, 6, 14)` — panics on invalid input. For tests and known-good literals.
- `Date::try_new(2025, 6, 14) -> Result<Date>` — for user input and parsing.
- `new()` is just `try_new().expect("invalid date")`.

**Reasoning:** Panicking constructors are idiomatic Rust for "this should never fail with valid literals" (cf. `Regex::new` debate). Fallible constructors are needed for any user-facing input path.

### 2.5 No nil date sentinel

**Decision:** Use `Option<Date>` instead of a sentinel nil value.

**Reasoning:** QuantMath uses `Date(0)` as a nil sentinel (e.g., in `DateDayFraction::start()`). This creates ambiguity — "is this date real or nil?" — and requires `is_valid()` checks. `Option<Date>` makes the optionality explicit in the type system. More Rustic, eliminates a class of bugs.

### 2.6 No date range validation in the type

**Decision:** No restriction on date range. jiff validates calendar correctness (no Feb 30). Business-logic range checks belong at a higher level.

**Reasoning:** QuantMath restricts to ~1968–2516. This is a business rule, not a type invariant. Different applications may have different valid ranges. Keep the type general.

---

## 3. Architecture: Cash-Flow-First

### 3.1 The insight

Every financial instrument ultimately produces cash flows. A bond is coupon payments + principal. A swap is two legs of cash flows. An option is a contingent cash flow. A future is daily margin cash flows.

QuantMath skips this abstraction — it goes directly from `Instrument` to pricing formulas. This works for pricing but fails for trade management, where you need to:
- Project future cash flows for funding/liquidity
- Match cash flows for reconciliation
- Net cash flows for compression
- Schedule settlements

### 3.2 Decision: Model `CashFlow` as a first-class type

```
CashFlow { amount: Decimal, currency: Arc<Currency>, pay_date: Date }
```

The atomic unit of finance. Every instrument can generate a `Vec<CashFlow>` (possibly contingent on market data). Present value of any cash flow is `amount * df(pay_date)`.

Two implementation decisions follow from this: coupon schedules are generated **backward from maturity** (short first stub — the market-standard convention, and one convention everywhere rather than per-pricer reimplementations), and contractual amounts are computed **exactly in `Decimal`** via `DayCount::accrue` (division last), because reconciliation cannot tolerate f64 noise in a coupon that should be exactly 2.50.

### 3.3 Build order: financial dependency

```
Layer 0: Foundation    — Error, Date, Timestamp, Zoned
Layer 1: Date Logic    — Calendar, DateRule, DayCount, Compounding
Layer 2: Reference     — Currency, CreditEntity, RateIndex
Layer 3: Cash Flows    — CashFlow, CashFlowSchedule
Layer 4: Curves        — DiscountCurve (parameterized by DayCount)
Layer 5: Instruments   — swaps (fixed/float legs), then futures, options
Layer 6: Forward Curves — commodity forward term structure
Layer 7: Pricing       — valuation using curves + market data
```

Each layer depends only on layers below it. This is different from QuantMath which builds instruments first and pricing second, with no explicit cash flow layer.

---

## 4. Currency & Reference Data

### 4.1 Decision: Currency is reference data, not an instrument

**QuantMath:** `Currency` is a variant of the `Instrument` enum. When "priced," it returns 1.0 (a dollar is always worth a dollar). Other instruments hold `Arc<Currency>`.

**qloxide:** `Currency` is reference data — a unit of account that instruments are denominated in. It does not appear on a trade blotter. You don't "trade" USD; you trade things denominated in USD.

```rust
pub struct Currency {
    pub id: String,
    pub settlement: DateRule,
    pub day_count: DayCount,    // for interest calculations in this currency
}
```

### 4.2 Reasoning

- Trade management needs clear separation between "the thing I traded" (instrument) and "the unit it's priced in" (currency)
- FX is its own instrument type (a currency *pair*), not a currency itself
- Currency deposits / money market instruments are specific instrument types, not "currency as instrument"
- CreditEntity similarly moves to reference data — it's a counterparty identity for curve lookups, not a tradeable thing

### 4.3 Other reference data types

| Type | Purpose |
|------|---------|
| `CreditEntity` | Counterparty/issuer identity for credit curve lookups |
| `RateIndex` | Defines a floating rate: id, currency, tenor, day_count, compounding, fixing_source (e.g., "USD-SOFR-3M") |
| `Calendar` | Holiday schedule per financial center |
| `DayCount` | Convention enum: Act360, Act365Fixed, Thirty360, ActActIsda. (BUS/252 deliberately omitted — it counts business days and needs a calendar parameter; add as `Bus252 { calendar }` when BRL instruments are needed.) |
| `Compounding` | Convention enum: Continuous, Simple, Annual, SemiAnnual, Daily |

---

## 5. Interest Rate Curves

### 5.1 Key insight: all curve types are the same math

All interest rate / credit / funding curves answer: "What is 1 unit of currency at future date T worth today?" The answer is a discount factor df(T). Different names reflect different credit entities or purposes, not different mathematics.

| Term | What it is | Formula |
|------|-----------|---------|
| **Discount curve** | df(T) directly | `df(T)` |
| **Yield/zero curve** | Continuously compounded rate | `r(T) = -ln(df(T)) / T` |
| **Forward rate** | Implied rate between T₁ and T₂ | `f = -ln(df(T₂)/df(T₁)) / (T₂-T₁)` |
| **Funding curve** | Your cost of borrowing | Same math, your credit |
| **Credit/hazard curve** | Counterparty default probability | Same math, survival probability |
| **Borrow curve** | Security lending cost (repo) | Same math, for forward pricing |

### 5.2 Decision: One `DiscountCurve` type, parameterized by DayCount

**QuantMath:** One `RateCurve` type with Act/365 hardcoded. Stores `(Date, yield)` pillar points, linear interpolation.

**qloxide:** Same mathematical structure, but parameterized:
```rust
pub struct DiscountCurve {
    base_date: Date,
    day_count: DayCount,
    pillars: Vec<(Date, f64)>,  // (date, continuously_compounded_yield)
    // + interpolation config
}
```

Methods:
- `df(from, to) -> f64` — discount factor between two dates
- `zero_rate(date) -> f64` — continuously compounded yield
- `forward_rate(from, to) -> f64` — implied forward rate
- `rt(date) -> f64` — rate * time (internal, same as QuantMath)

Identity (whose curve, what purpose) comes from how it's stored in market data:
```rust
struct MarketCurves {
    discount: HashMap<CurrencyId, DiscountCurve>,    // risk-free per currency
    credit:   HashMap<CreditId, DiscountCurve>,      // counterparty survival
    borrow:   HashMap<InstrumentId, DiscountCurve>,   // repo/stock loan
}
```

### 5.3 What QuantMath is missing

| Gap | qloxide approach |
|-----|-----------------|
| Hardcoded Act/365 | `DayCount` enum parameter on curve |
| No compounding conventions | `Compounding` enum for rate conversions |
| No curve bootstrapping | Deferred — curves input as pillar points initially |
| No rate index definitions | `RateIndex` reference type for floating legs |

### 5.4 Commodity forward curves

Commodity forward curves are NOT derived from spot + interest rates. They come directly from futures prices:
```
Brent: { Jun25: 72.50, Jul25: 72.10, Aug25: 71.80, ... }
```
This is a separate market data type, built after interest rate curves (Layer 6).

---

## 6. Instrument Polymorphism

### 6.1 Decision: Trait-based with `typetag`

```rust
#[typetag::serde(tag = "type")]
pub trait FinancialInstrument: Send + Sync + Debug {
    fn id(&self) -> &str;
    fn currency(&self) -> &Currency;
    fn settlement(&self) -> &Settlement;
    fn maturity(&self) -> Option<Date>;
    fn instrument_type(&self) -> &str;
    fn as_any(&self) -> &dyn Any;
}

#[derive(Serialize, Deserialize)]
pub struct Future {
    pub id: String,
    pub underlying: String,
    pub currency: Arc<Currency>,
    // ...
}

#[typetag::serde]
impl FinancialInstrument for Future {
    fn id(&self) -> &str { &self.id }
    fn currency(&self) -> &Currency { &self.currency }
    fn settlement(&self) -> &Settlement { &self.settlement }
}
```

JSON output:
```json
{"type": "Future", "id": "ICE-B-Jun25", "underlying": "Brent", ...}
```

### 6.2 Reasoning

**Requirements that drove the decision:**
1. **Clean caller interface** — `instrument.id()`, not match arms at every call site
2. **Runtime extensibility** — add new instrument types without modifying existing code (recompilation required, but zero changes to existing files)
3. **JSON serialization** — instruments must round-trip through JSON for trade capture, blotter, API
4. **Basket composition** — instruments must compose recursively: `Vec<(f64, Arc<dyn FinancialInstrument>)>`

**Alternatives considered:**

| Approach | Why rejected |
|----------|-------------|
| **Flat enum (QuantMath)** | No runtime extensibility. Adding a type touches the enum + every match arm. |
| **Pure trait + erased_serde** | ~200 lines of boilerplate for type registry. Fragile runtime errors. |
| **Nested enum** | Still closed. Category boundaries are debatable (where does a bond go?). |
| **Struct + kind enum** | Loses type-level distinction. Some fields don't apply to all kinds. |
| **Enum + trait hybrid** | Would give both, but redundant — typetag solves the serde problem cleanly. |

**Why `typetag`:**
- By dtolnay (serde author) — well-maintained, production-quality
- Automatic tagged serialization for trait objects
- Zero boilerplate beyond `#[typetag::serde]` on each impl
- Works with `Arc<dyn Trait>`, `Box<dyn Trait>`

### 6.3 Settlement sessions in instruments

Instead of QuantMath's bare `TimeOfDay { Open, EDSP, Close }`, settlement timing is an attribute of the instrument/contract definition:

```rust
pub struct Settlement {
    pub venue: String,              // "ICE", "CME", "PLATTS"
    pub session: String,            // "SETTLE", "SINGAPORE_CLOSE"
    pub time: Time,                 // 19:30 (typed, minute precision)
    pub timezone: String,           // "Europe/London" — IANA-validated at deserialization
    pub payment_lag: DateRule,      // T+N business days until cash moves
}
```

A contract says "ICE SETTLE at 19:30 Europe/London" — sufficient for both pricing (convert to day fraction) and operations (reconciliation, blotter matching). `at_date(date)` resolves the settlement instant as a `Zoned`; `pay_date(date)` applies the payment lag.

---

## 7. Domain Model Summary

```
┌─────────────────────────────────────────────────┐
│  Reference Data (slowly changing)               │
│  Currency, CreditEntity, RateIndex, Calendar,   │
│  DayCount, Compounding                          │
└──────────────────┬──────────────────────────────┘
                   │
┌──────────────────▼──────────────────────────────┐
│  Financial Instruments (trait-based, typetag)    │
│  trait FinancialInstrument { id, currency, ... } │
│  Implementations: Future, Swap, Option, Bond,   │
│  Basket, Equity, FX, ...                        │
└──────────────────┬──────────────────────────────┘
                   │
┌──────────────────▼──────────────────────────────┐
│  Cash Flows (generated by instruments)          │
│  CashFlow { amount, currency, pay_date }        │
│  CashFlowSchedule (date generation + flows)     │
└──────────────────┬──────────────────────────────┘
                   │
┌──────────────────▼──────────────────────────────┐
│  Deals/Trades (immutable events)                │
│  Deal { instrument_id, direction: BuySell,      │
│         quantity: Decimal, price: Decimal,       │
│         timestamp, counterparty, venue }        │
└──────────────────┬──────────────────────────────┘
                   │
┌──────────────────▼──────────────────────────────┐
│  Positions (derived from deals)                 │
│  Position { instrument_id, direction, quantity, │
│             avg_price } — via portfolio::compress│
└──────────────────┬──────────────────────────────┘
                   │
┌──────────────────▼──────────────────────────────┐
│  Market Data & Curves                           │
│  DiscountCurve, CommodityForwardCurve,          │
│  VolSurface, SpotPrices, Fixings                │
└──────────────────┬──────────────────────────────┘
                   │
┌──────────────────▼──────────────────────────────┐
│  Pricing & Risk                                 │
│  Valuation, Greeks, Scenarios                   │
└─────────────────────────────────────────────────┘
```

---

## 8. Pricing Architecture

### 8.1 The problem with QuantMath's approach

QuantMath defines `Priceable` and `MonteCarloPriceable` as traits **in the instruments module**. Each instrument implements them directly — e.g., `SpotStartingEuropean::prices()` calls Black76 internally. This creates two problems:

1. **Instruments depend on market data types** — the `instruments` module imports curves, vol surfaces, forwards. You can't use instrument definitions without pulling in the entire pricing stack. This blocks non-pricing use cases (trade reconciliation, portfolio compression, blotter matching) from being lightweight.

2. **Model is hardcoded in the instrument** — Black76 is baked into option pricing. To use Bachelier (normal model, standard for interest rate options) or SABR (for swaptions), you'd need to either add model-selection logic inside the instrument or create duplicate instrument types per model. Neither is right — the instrument is the same contract regardless of model.

### 8.2 Decision: Two explicit inputs; model implicit in the vol surface

> **Superseded design note (2026-06):** an earlier revision of this section specified three inputs — `price(instrument, context, Option<&Model>)` with a `Model` enum. That was superseded during options planning; what follows is what the code implements:

```
Instrument     = what is the contract (strike, expiry, put/call)
PricingContext = what does the market say (market prices, curves, vol surfaces)
```

The pricing interface is uniform: `price(instrument, context) -> Result<f64>` for every instrument type. There is no separate model parameter — the **model choice is carried by the vol surface variant**: `VolSurface::Flat` (lognormal vol) implies Black76; a future `FlatBachelier` (normal vol) implies Bachelier. This matches market practice — commodity option quotes are natively in the model's vol units, so the surface type genuinely encodes the model.

**Tradeoff:** this folds methodology into market data, which §8.4's original argument kept separate. It is the right call for the current trajectory (Black76/Bachelier on commodities). Revisit if calibrated models arrive (SABR, local vol) — a calibrated model is *not* observable market data and will likely need to become an explicit object again.

### 8.3 Decision: Pricing is a separate module, not on instruments

`Priceable` is a standalone trait in `src/pricing/`, **not** a supertrait of `FinancialInstrument`. Instruments remain pure data definitions.

**Why:** Instruments are usable for reconciliation, compression, trade capture, and serialization without any pricing dependency. Adding new market data types or models never touches the instruments module.

**Tradeoff:** Can't call `.price()` on `dyn FinancialInstrument`. Instead, a dispatch function in the pricing module downcasts via `as_any()`:

```rust
// src/pricing/mod.rs
fn price(inst: &dyn FinancialInstrument, ctx: &dyn PricingContext) -> Result<f64> {
    let any = inst.as_any();
    if let Some(b) = any.downcast_ref::<Bond>() { return price_bond(b, ctx); }
    if let Some(f) = any.downcast_ref::<Future>() { return price_future(f, ctx); }
    if let Some(o) = any.downcast_ref::<EuropeanOption>() { return price_european(o, ctx); }
    // ...
    Err(Error::Pricer(format!("no pricer for {}", inst.instrument_type())))
}
```

This is the one closed-dispatch point — but it's in the pricing module where you'd write the pricing logic anyway. Adding a new instrument means one pricing function + one match arm.

### 8.4 PricingContext is market data

**PricingContext is market data** — market prices, curves, vol surfaces, fixings. Observable inputs. Two people looking at the same market see the same PricingContext. A scenario / VaR run is just a different PricingContext with shifted inputs; everything reprices uniformly through the same `price()` function.

```rust
// src/pricing/mod.rs (implemented)
trait PricingContext: Send + Sync {
    fn as_of(&self) -> Timestamp;
    fn spot_date(&self) -> Date;   // explicit — no UTC-derived default (timezone trap)
    fn discount_curve(&self, currency: &str) -> Result<&DiscountCurve>;
    fn spot(&self, id: &str) -> Result<f64>;
    fn settlement_price(&self, id: &str) -> Result<f64>;
    // vol(id, tenor, moneyness) arrives with the options work
}
```

### 8.5 Deterministic vs model-dependent instruments

Some instruments' prices are fully determined by market observables; options additionally need a vol surface (which carries the model, §8.2):

| Instrument | Pricing formula | Needs vol surface? |
|------------|----------------|--------------------|
| Bond | Sum of `cash_flows()` discounted: `amount * df(pay_date)` | No |
| Future | Market price (mark), settlement price after expiry | No |
| Swap | Fixed leg PV minus floating leg PV | No |
| FxForward | Spot * df ratio | No |
| EuropeanOption | Black76 (or Bachelier) from the surface | Yes |
| Basket | Weighted sum of components | Depends on components |

This is QuantMath's "self-pricing" concept — but the formula lives in the pricing module, not on the instrument, and there is no API distinction: every instrument prices through `price(instrument, context)`; an option simply errors if the context has no vol surface for its underlying.

### 8.6 Deal valuation

Deal valuation lives in `portfolio::valuate`: it looks up the instrument by `instrument_id`, delegates to `pricing::price`, and produces one `ValuedDeal` per input deal — pricing failures are carried as errors, never silently dropped. P&L = `signed_quantity * (mark - trade price) * contract_size`.

Decimal for deal-level P&L (exact), f64 inside pricing math (performance).

## 9. Comparison with QuantMath

| Aspect | QuantMath | qloxide |
|--------|-----------|---------|
| Primary purpose | Pricing library | Instrument modeling + trade management + pricing |
| Instrument model | Closed enum (7 variants) | Open trait (`typetag`) |
| Currency | Instrument variant (prices at 1.0) | Reference data |
| CreditEntity | Instrument variant (prices at 1.0) | Reference data |
| Cash flow type | None (implicit via ZeroCoupon) | First-class `CashFlow` struct |
| Time types | `Date(i32)` + `TimeOfDay { Open, EDSP, Close }` | `Date`, `Timestamp`, `Zoned` (jiff) |
| Settlement | `DateRule` only | `DateRule` + `Settlement` (venue + time + timezone) |
| Day count | Hardcoded Act/365 | `DayCount` enum (Act360, Act365, 30/360, etc.) |
| Compounding | Continuously compounded only | `Compounding` enum (continuous, simple, annual, etc.) |
| Rate index | Not modeled | `RateIndex` reference type |
| Commodity forwards | Not supported (equity-only model) | Separate forward curve from futures prices |
| Curve construction | Pre-cooked pillar points | Same initially, bootstrapping later |
| Serde | Derive on enum (simple) | `typetag` on trait (extensible) |
| Pricing location | Traits on instruments (instruments depend on market data) | Separate pricing module (instruments are pure data) |
| Model choice | Black76 hardcoded in instrument | Implicit in vol surface variant (Flat = Black76, FlatBachelier = Bachelier) |
| Pricing inputs | `instrument.price(context)` | `price(instrument, context)` — uniform for all instrument types |

---


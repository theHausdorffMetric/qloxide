# Options on Futures — Implementation Plan

## Context

qloxide is a Rust financial library focused on energy/commodities. Layers 0–5.5 are complete (dates, reference data, cash flows, curves, instruments, trades). Deterministic pricing works for bonds and futures. The Brent crude example has 12 futures, 4 deals, and working P&L reports.

The goal is to price **European options on futures** using the **Black76 model** (the b=0 specialization of generalized Black-Scholes-Merton). This is Phase 1 of a larger options pricing effort. Reference formulas exist in `docs/opt_src/gbsm.rs` with tested values.

## Pricing Architecture

All pricing goes through one uniform interface:

```
price(instrument, context) -> Result<f64>
```

There is no distinction between "self-pricing" and "model-dependent" instruments. Every instrument is priced the same way — the `PricingContext` provides all market data (market prices, curves, vol surfaces), and each instrument type takes what it needs:

- **Future**: needs `ctx.market_price(id)`
- **Bond**: needs `ctx.market_price(id)` + `ctx.discount_curve(ccy)`
- **EuropeanOption**: needs `ctx.market_price(underlying)` + `ctx.discount_curve(ccy)` + `ctx.vol(underlying, tenor, moneyness)`

The **model choice** (Black76 vs Bachelier) is implicit in the vol surface type. A `VolSurface::Flat { vol: 0.30 }` implies lognormal (Black76). Future variants like `VolSurface::FlatBachelier { vol: 20.0 }` will imply normal (Bachelier). No separate model parameter is needed.

**Market price vs model price:** `ctx.market_price(id)` returns the observed/exchange price for any instrument (futures, listed options). `price(instrument, ctx)` returns the computed/theoretical price (e.g. Black76 for options). They serve different purposes — market price for P&L/accounting, model price for Greeks/risk/fair value.

This design also supports scenario analysis / VaR: a scenario is just a different `PricingContext` with shifted market prices, curves, and vols. Everything reprices uniformly through the same `price()` function.

### Naming: `spot` → `market_price`, `spot_date` → `valuation_date`

The existing codebase uses `spot` and `spot_date`. These are renamed as part of this phase:

- `spot(id)` → `market_price(id)` — "spot" in commodities means prompt physical delivery, not "current price of any instrument"
- `spot_date` → `valuation_date` — "spot date" in FX means T+2 settlement; here it's just the pricing/valuation date
- `spots` (HashMap) → `market_prices`
- `settlement_price(id)` stays — it's the correct term for the final settlement of an expired instrument

This rename touches: `MarketData`, `PricingContext` trait, all pricers (`bond.rs`, `future.rs`), `portfolio.rs`, `reports.rs`, `config.rs`, and the example JSON files (`market.json`).

## Overall Roadmap

| Phase | Scope | Status |
|-------|-------|--------|
| **1 — Black76** | Math module, Black76 pricing + Greeks, vol surface, European option pricer, Brent option example | **This plan** |
| 2 — Implied vol | Brent root finder, ivol from market prices | Future |
| 3 — Bachelier | Normal model (add `FlatBachelier` vol surface variant, dispatch in european pricer) | Future |
| 4 — Asian (TW) | Turnbull-Wakeman for average price options | Future |
| 5 — Scenarios | Scenario engine: reprices portfolio under shifted PricingContexts (historical VaR, stress) | Future |

## Phase 1 — Detailed Implementation

### Step 0: Rename `spot` → `market_price`, `spot_date` → `valuation_date`

Rename across the entire codebase before adding new code. This is a mechanical find-and-replace.

**Files to modify:**
- `src/market_data/mod.rs` — struct field `spots` → `market_prices`, `spot_date` → `valuation_date`, all methods
- `src/pricing/mod.rs` — `PricingContext` trait methods, `MarketData` impl
- `src/pricing/future.rs` — calls to `ctx.spot()` → `ctx.market_price()`
- `src/pricing/bond.rs` — (if it uses spot_date)
- `src/portfolio.rs` — calls to `md.spot_date()`, `md.spot()`
- `src/config.rs` — consistency checks
- `src/reports.rs` — status/price lookups
- `examples/brent/market.json` — `"spots"` → `"market_prices"`, `"spot_date"` → `"valuation_date"`
- `examples/bond_pricing/bond_pricing_market.json` — same renames
- `tests/instrument_json.rs` — if it references spot data

Run `cargo test` after to confirm no breakage.

### Step 1: Add `libm` dependency

**File:** `qloxide/Cargo.toml`

Add `libm = "0.2"` to `[dependencies]`. Provides `erf()` for `norm_cdf`. Tiny, pure Rust, no-std compatible. Same library used in the reference code.

### Step 2: Create math module

**New file:** `qloxide/src/math/mod.rs`
**Modify:** `qloxide/src/lib.rs` — add `pub mod math;`

Contents:
```rust
norm_pdf(x: f64) -> f64    // 1/√(2π) · e^(-x²/2)
norm_cdf(x: f64) -> f64    // 0.5 · (1 + erf(x/√2))
```

Tests: `norm_pdf(0) ≈ 0.3989`, `norm_cdf(0) = 0.5`, symmetry `norm_cdf(-x) + norm_cdf(x) = 1`.

### Step 3: Create Black76 pricing module

**New file:** `qloxide/src/pricing/black76.rs`
**Modify:** `qloxide/src/pricing/mod.rs` — add `pub mod black76;`

Pure math functions taking f64 inputs. No instrument/market data types.

**Structs:**
- `Black76Params { f, k, t, r, sigma }` — input parameters
- `Greeks { price, delta, gamma, vega, theta }` — output (public, reusable)

**Functions:**
- `black76_price(params, put_or_call) -> Result<f64>` — validates inputs, returns option premium
- `black76_greeks(params, put_or_call) -> Result<Greeks>` — validates inputs, returns all Greeks

**Black76 formulas (b=0):**
```
d1 = [ln(F/K) + (σ²/2)·T] / (σ·√T)
d2 = d1 - σ·√T
Call = e^(-rT) · [F·N(d1) - K·N(d2)]
Put  = e^(-rT) · [K·N(-d2) - F·N(-d1)]
Delta_call = e^(-rT) · N(d1),  Delta_put = e^(-rT) · (N(d1) - 1)
Gamma = e^(-rT) · n(d1) / (F·σ·√T)
Vega  = F · e^(-rT) · n(d1) · √T
Theta_call = -(F·e^(-rT)·n(d1)·σ)/(2√T) - r·K·e^(-rT)·N(d2)
Theta_put  = -(F·e^(-rT)·n(d1)·σ)/(2√T) + r·K·e^(-rT)·N(-d2)
```

**Edge cases:** T·σ < 1e-10 → return discounted intrinsic. F ≤ 0, K ≤ 0, σ < 0, T < 0 → error.

**Tests (from reference code `docs/opt_src/gbsm.rs`):**
- `Call(F=100, K=100, T=1, r=0.01, σ=0.3) = 11.80489728393353` (tol 1e-10)
- `Delta(Call, F=105, K=100, T=0.5, r=0.1, σ=0.36) ≈ 0.5946` (tol 1e-4)
- `Delta(Put, F=105, K=100, T=0.5, r=0.1, σ=0.36) ≈ -0.3566` (tol 1e-4)
- Put-call parity: `C - P = e^(-rT) · (F - K)`
- Zero vol / zero time → discounted intrinsic
- Vega always ≥ 0, Gamma always ≥ 0

### Step 4: Add VolSurface and volatility to MarketData

**Design decision:** Vol is a function of tenor and moneyness, not a flat number. The interface is right from the start; Phase 1 implements `FlatVol` as the only variant.

**New file:** `qloxide/src/market_data/vol.rs`

```rust
/// Volatility surface — returns vol as f(tenor, moneyness).
///
/// Moneyness = ln(K/F), tenor = time to expiry in years.
/// Phase 1: FlatVol ignores both parameters.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum VolSurface {
    /// Constant vol regardless of tenor/moneyness.
    Flat { vol: f64 },
    // Future variants:
    // Grid { tenors, strikes, vols } — interpolated surface
    // SABR { alpha, beta, rho, nu } — parametric
}

impl VolSurface {
    pub fn vol(&self, _tenor: f64, _moneyness: f64) -> f64 {
        match self {
            VolSurface::Flat { vol } => *vol,
        }
    }
}
```

**Modify:** `qloxide/src/market_data/mod.rs`
- Add `pub mod vol;`
- Add field: `#[serde(default)] vol_surfaces: HashMap<String, VolSurface>`
- Add methods: `vol_surface(id) -> Result<&VolSurface>`, `add_vol_surface(id, surface)`
- Update `merge()` to handle vol_surfaces (reject duplicates)
- Tests: add/retrieve vol surface, missing surface errors

**Modify:** `qloxide/src/pricing/mod.rs`
- Add to `PricingContext` trait:
  ```rust
  fn vol(&self, id: &str, tenor: f64, moneyness: f64) -> core::Result<f64>;
  ```
- Implement in `impl PricingContext for MarketData`:
  ```rust
  fn vol(&self, id: &str, tenor: f64, moneyness: f64) -> core::Result<f64> {
      Ok(self.vol_surface(id)?.vol(tenor, moneyness))
  }
  ```
- Update `EmptyContext` in tests

### Step 5: Wire up the European option pricer

**New file:** `qloxide/src/pricing/european.rs`
**Modify:** `qloxide/src/pricing/mod.rs` — add `pub mod european;`, add dispatch arm

Bridge between EuropeanOption instrument, PricingContext, and Black76 math:

```
price_european(option, ctx):
  1. f = ctx.market_price(option.underlying)               // futures price
  2. k = option.strike.to_f64()                    // strike
  3. t = currency.day_count.year_fraction(valuation_date, expiry)  // tenor
  4. r = ctx.discount_curve(currency).zero_rate(expiry)       // rate
  5. moneyness = ln(k / f)                         // log-moneyness
  6. sigma = ctx.vol(option.underlying, t, moneyness)  // vol surface lookup
  7. return black76_price({f, k, t, r, sigma}, option.put_or_call)
```

If t ≤ 0 (expired): return intrinsic value.

Also `greeks_european(option, ctx) -> Result<Greeks>` with same setup.

Move `decimal_to_f64` from `pricing/bond.rs` to `pricing/mod.rs` as `pub(crate)` (shared by bond and european pricers).

**Add dispatch in `price()`:**
```rust
if let Some(o) = any.downcast_ref::<EuropeanOption>() {
    return european::price_european(o, ctx);
}
```

Tests: known-value pricing through the full dispatch chain, expired option, missing vol/market_price errors.

### Step 6: Option P&L in dollar terms

**Modify:** `qloxide/src/portfolio.rs`

The `valuate` function currently gets `contract_size` from the underlying `Future`. For options, look up the underlying future's contract_size so option P&L is in dollar terms, consistent with futures:

```rust
let contract_size = inst.as_any().downcast_ref::<Future>()
    .map(|f| f.contract_size)
    .or_else(|| {
        // For options, get contract_size from the underlying future
        inst.as_any().downcast_ref::<EuropeanOption>()
            .and_then(|o| portfolio.instruments.get(&o.underlying))
            .and_then(|u| u.as_any().downcast_ref::<Future>())
            .map(|f| f.contract_size)
    })
    .unwrap_or(Decimal::ONE);
```

This means a 10-lot option deal with premium change of $0.50/bbl on 1000-bbl contracts = $5,000 P&L.

### Step 7: Add Brent option to example

**New file:** `qloxide/examples/brent/options.json`
- One European call on ICE-B-K26, strike $75, expiry 2026-03-26

**Modify:** `qloxide/examples/brent/market.json`
- Add vol surface:
  ```json
  "vol_surfaces": {
      "ICE-B-K26": { "type": "Flat", "vol": 0.30 }
  }
  ```

**Modify:** `qloxide/examples/brent/brent.toml`
- `instruments = ["instruments.json", "options.json"]`

**Modify:** `qloxide/examples/brent/deals.json`
- Add option deal: Buy 10 lots of ICE-BUL-K26-C-75 @ $2.50

### Step 8: Update config checks and reports for options

**Modify:** `qloxide/src/config.rs`
- Add warnings for options with missing underlying instrument or missing vol surface

**Modify:** `qloxide/src/reports.rs`
- Replace `Future`-specific downcast for expiry with `inst.maturity()` (works for all types)
- Same for expired check: use `inst.maturity().is_some_and(|m| valuation_date > m)`

## Files Summary

| File | Action | What |
|------|--------|------|
| _Step 0: Rename_ | | |
| `src/market_data/mod.rs` | Modify | `spots` → `market_prices`, `spot_date` → `valuation_date`, all methods |
| `src/pricing/mod.rs` | Modify | `PricingContext` trait: `spot()` → `market_price()`, `spot_date()` → `valuation_date()` |
| `src/pricing/future.rs` | Modify | Update calls |
| `src/pricing/bond.rs` | Modify | Update calls |
| `src/portfolio.rs` | Modify | Update calls |
| `src/config.rs` | Modify | Update calls |
| `src/reports.rs` | Modify | Update calls |
| `examples/brent/market.json` | Modify | `"spots"` → `"market_prices"`, `"spot_date"` → `"valuation_date"` |
| `examples/bond_pricing/bond_pricing_market.json` | Modify | Same renames |
| _Step 1+: Options_ | | |
| `Cargo.toml` | Modify | Add `libm = "0.2"` |
| `src/lib.rs` | Modify | Add `pub mod math;` |
| `src/math/mod.rs` | **New** | `norm_pdf`, `norm_cdf` |
| `src/pricing/black76.rs` | **New** | `Black76Params`, `Greeks`, `black76_price`, `black76_greeks` |
| `src/pricing/european.rs` | **New** | `price_european`, `greeks_european` |
| `src/pricing/mod.rs` | Modify | Add `vol()` to PricingContext, dispatch for EuropeanOption, shared `decimal_to_f64` |
| `src/pricing/bond.rs` | Modify | Use shared `decimal_to_f64` from mod.rs |
| `src/market_data/vol.rs` | **New** | `VolSurface` enum (`Flat` variant), `vol(tenor, moneyness)` method |
| `src/market_data/mod.rs` | Modify | Add `vol_surfaces` HashMap, lookup methods, merge support |
| `src/portfolio.rs` | Modify | Look up underlying future's contract_size for option P&L in dollar terms |
| `src/config.rs` | Modify | Option-specific consistency warnings |
| `src/reports.rs` | Modify | Use `maturity()` instead of Future downcast |
| `examples/brent/options.json` | **New** | Brent call option instrument |
| `examples/brent/market.json` | Modify | Add vol_surfaces |
| `examples/brent/brent.toml` | Modify | Include options.json |
| `examples/brent/deals.json` | Modify | Add option deal |

## Key Design Decisions

1. **Uniform pricing interface.** `price(instrument, context) -> Result<f64>` for all instrument types. No self-pricing vs model-dependent distinction. Context provides everything.
2. **Model choice is implicit in the vol surface.** `VolSurface::Flat` = lognormal = Black76. Future `FlatBachelier` variant = normal = Bachelier. No separate model parameter.
3. **Vol is a surface, not a scalar.** `VolSurface::vol(tenor, moneyness) -> f64`. Phase 1 implements `Flat` only; the interface is ready for `Grid`, `SABR`, etc.
4. **Moneyness = ln(K/F).** Log-moneyness is the standard parameterization. The european pricer computes this from strike and underlying spot.
5. **Vol keyed by underlying ID** (e.g. `"ICE-B-K26"`). Each futures contract month has its own vol surface.
6. **Option P&L in dollar terms.** Multiplied by the underlying future's contract_size (1000 bbl for Brent). Consistent with futures P&L.
7. **Scenario-ready.** A scenario is just a different `PricingContext`. Everything reprices uniformly — no special cases for different instrument types.

## Not in Scope

- Vol surface interpolation (strike × term) — Phase 1 uses Flat only
- Bachelier model — Phase 3 (adds `FlatBachelier` vol surface variant)
- Implied volatility solver — Phase 2
- American options — needs binomial tree
- Scenario engine / VaR — Phase 5 (but architecture supports it)

## Verification

1. `cargo test` — all existing 117 tests pass + new tests for math, black76, european pricer, vol surface
2. `cargo run -- --config examples/brent/brent.toml` — Brent example shows the option priced in instruments report and dollar P&L
3. Validate Black76 price against reference: `Call(F=100, K=100, T=1, r=0.01, σ=0.3) = 11.80489728`
4. Verify put-call parity holds through the full pricing chain
5. Verify option P&L is in dollar terms (premium × contract_size × signed_qty)

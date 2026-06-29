# qloxide: Status Review, Design & Code Evaluation — 2026-06-12

## 1. Project Status

| Area | Status |
|------|--------|
| Layers 0–5.5 (dates, reference data, cash flows, curves, instruments, trades) | Implemented |
| Deterministic pricing (Bond, Future) | Implemented, timestamp-aware expiry for futures |
| Config loader, CLI binary, portfolio reports | Implemented; Brent example (12 futures, 4 deals) runs end to end |
| Tests | 117 passing (105 unit + 12 integration), zero failures |
| Clippy | 4 trivial warnings (empty format strings, manual range-contains) |
| Options plan (`docs/options-plan.md`, Phase 1 Black76) | Written, **not started** — including the `spot` → `market_price` rename (Step 0) |
| Swap/Equity/FxForward/Basket pricers | Not implemented (`price()` returns "no pricer") |
| Layer 6 (commodity forward curves) | Not started |
| Uncommitted work | qloxide: 6 files (Deal `venue` removal, settlement time validation, day-count bond tests). Parent repo: doc updates, `docs/opt_src/`, `docs/options-plan.md`, QuantMath submodule pointer |

## 2. Design Approach Evaluation

### What is genuinely good

- **The "why not QuantMath" analysis is sound.** Each divergence (DayCount enum vs hardcoded Act/365, Currency as reference data, Settlement with venue/session/time/timezone, first-class CashFlow, open trait vs closed enum) addresses a concrete deficiency for the energy/commodities + trade-management use case, and the docs record the alternatives considered. This is unusually disciplined design documentation.
- **typetag for instrument polymorphism** is the right call for the stated requirements (clean caller interface, JSON round-trip, recursive baskets, extensibility). The one closed dispatch point (`pricing::price`) lives exactly where new code must be written anyway.
- **Three jiff newtypes** (Date / Timestamp / Zoned) cleanly separate "which day", "which instant", "which wall-clock time where". The futures pricer already exploits this (intraday expiry checks against the settlement session) — a capability QuantMath cannot express.
- **Pricing as a separate module with instruments as pure data** keeps the trade-management use cases (reconciliation, compression, blotter) dependency-light, as intended.
- **Decimal/f64 boundary policy** (Decimal for deal quantities/prices/P&L, f64 inside pricing math) is the standard, correct trade-off and is applied consistently.
- **Layer discipline holds in the code**: module dependencies match the documented layer order; no backward dependencies found.

### Design problems

**D1. The architecture doc contradicts the options plan on the pricing interface.**
`qloxide-architecture.md` §9 specifies `price(instrument, context, model: Option<&Model>)` with a `Model` enum; `options-plan.md` supersedes this with `price(instrument, context)` and model-implicit-in-vol-surface (`VolSurface::Flat` ⇒ Black76, `FlatBachelier` ⇒ Bachelier). The implemented code matches the options plan. The model-in-surface choice is defensible for the stated trajectory — commodity option quotes are natively in the model's vol units, so the surface type genuinely carries the model — but it partially conflates §9.4's "context = observable data, model = methodology" separation. Whichever way, **one document must win**; right now §9 describes an API that will never exist. Action: rewrite §9 to match the options plan and record the trade-off (revisit if SABR/calibrated models arrive, since a calibrated model is *not* market data).

**D2. The cash-flow-first architecture is asserted but not wired.**
§3 calls `CashFlow` "the atomic unit" and says every instrument can generate `Vec<CashFlow>`. In reality: no trait method or function produces `CashFlow` from any instrument; `CashFlowSchedule` is used by nothing outside its own tests; and `price_bond` hand-rolls its own coupon dates **forward from issue** while `CashFlowSchedule::generate` rolls **backward from maturity** — two different stub conventions in one codebase that will disagree for any bond whose tenor isn't a whole number of periods. The central architectural insight currently exists only as dead code plus a parallel reimplementation.

**D3. Positions diverge from the domain model.**
The architecture diagram has `Position { instrument, net_qty }` as its own derived type. `portfolio::compress` instead returns synthetic `Deal`s with fabricated ids (`NET-…`), an empty counterparty, and the timestamp of the *last deal overall* (not even the last deal in that instrument). The doc itself criticizes sentinel values (§2.5); empty-string counterparty and fake timestamps are sentinels. Also note the VWAP semantics: netting Buy 10 @ 71.80 / Sell 4 @ 72.50 yields avg 71.33 — that embeds realized P&L from the closed lots into the open-position price. That may be intended ("net entry price"), but it's a financial-semantics decision that deserves a sentence of documentation; most blotters show FIFO/avg cost of *remaining* lots instead.

**D4. Currency-by-value duplication has no consistency guard.**
Every instrument embeds its own `Currency` copy in JSON (the documented Arc/self-contained choice — fine), but nothing detects two instruments declaring `"USD"` with *different* `day_count` or settlement rules. For a reconciliation-oriented library this is exactly the silent-mismatch class it exists to kill. A cheap config-load check (same id ⇒ identical fields, else warning) closes it.

**D5. Stored derived data, still unvalidated.**
`EuropeanOption.pay_date` is stored and serialized alongside `settlement.payment_lag` which can compute it. The build plan flagged this exact issue at Step 7 ("if hand-edited JSON has inconsistent values, no validation. Acceptable?") and it remains open. Either drop the field (derive via `settlement.pay_date(expiry)`) or validate at load.

**D6. Speculative reference data.** `CreditEntity`, `RateIndex`, and instrument `credit_id` fields are defined but consumed by nothing. Acceptable as planned-layer scaffolding, but `credit_id: String` on five instruments is cost-free only until someone has to populate it meaningfully. Keep, but don't extend until a credit curve consumer exists.

## 3. Implementation Evaluation

### Correctness bugs (verified or high-confidence)

**B1 — `Calendar::Volatility` sign bug in `count_business_days`** (`src/dates/calendar.rs:120-122`). The recursive call already receives `(from, to)` in original order and returns a correctly signed count; multiplying by `sign.signum()` double-applies the sign and flips the result whenever `from > to`. Verified: `vol.count_business_days(later, earlier)` returns **+5** where the wrapped calendar returns **−5**. Fix: return `calendar.count_business_days(from, to)` unchanged.

**B2 — `DayCount::Act252` counts calendar days.** The doc (and the variant's own comment) defines it as Brazilian *business* days / 252; the implementation is `(to - from) as f64 / 252.0`. Real BUS/252 needs a calendar (cf. QuantLib's `Business252(calendar)`). Either implement `Bus252 { calendar }` (makes `DayCount` non-`Copy` — ripple), or remove/rename the variant until needed. Shipping a convention that's quietly wrong is the worst of the three options.

**B3 — `portfolio::valuate` silently drops unpriceable deals** (`src/portfolio.rs:69-72`, `filter_map` + `.ok()?`). A deal whose instrument is missing a spot/curve simply vanishes from the P&L report with no trace. For a risk/P&L tool, silently omitting positions is the most dangerous failure mode available. Fix: surface per-deal errors (e.g. `ValuedDeal { mark: Option<…>, error: Option<String> }` or return `(Vec<ValuedDeal>, Vec<String>)`) and have the pnl report print unpriced rows explicitly.

**B4 — `price_bond` panics or mispricess on bad `frequency`** (`src/pricing/bond.rs:14`). `12 / bond.frequency` panics on `frequency: 0` (reachable from JSON) and silently produces 6 periods/year for `frequency: 5`. The `cashflows::Frequency` enum already exists and makes both states unrepresentable — `Bond` should use it (pre-1.0 JSON break is acceptable), or at minimum validate `freq > 0 && 12 % freq == 0`.

**B5 — `PricingContext::spot_date` default impl is a timezone footgun** (`src/pricing/mod.rs:18-20`). The default derives the date from `as_of` in UTC, so a 19:00 New York snapshot yields *tomorrow's* spot date. `MarketData` overrides it so the bug is latent, but every future `PricingContext` impl inherits the trap. Remove the default (make both methods required) or document loudly.

**B6 — config array/object fallback masks real errors** (`src/config.rs:160-173`). A malformed element *inside* an array fails the `Vec` parse, falls through to single-object parsing, and reports "invalid instrument JSON … expected struct" — pointing the user at the wrong problem. Peek at the first non-whitespace byte (`[` vs `{`) and parse accordingly, so array errors surface as array errors.

**B7 — string-typed date comparison in reports** (`src/reports.rs:43-47`). `md.spot_date().to_string() > expiry` compares ISO strings (works, but fragile) and the `Future`-only downcast means bonds/options never show expiry. The options plan (Step 8) already prescribes `inst.maturity()` — do it now, it's a five-line fix.

### Idiomatic Rust

The codebase is, overall, idiomatic and pleasant: newtypes with targeted trait impls, `thiserror`, the `#[serde(from = "Raw")]` pattern for `DiscountCurve`'s cached `pillar_rts` (textbook), no `unsafe`, no `unwrap()` on fallible paths in library code (panicking ops are deliberate and documented), inline test modules with strong coverage habits. Remaining nits, roughly ordered:

- **`#![allow(clippy::too_many_arguments)]` at crate level** (`lib.rs:1`) suppresses a real smell globally. `Bond::new` takes 10 positional args, several of the same type — transposing `id`/`credit_id` compiles fine. Since all fields are `pub`, the cheapest fix is to prefer struct-literal construction in docs/tests and keep `new` only where short; scope the allow per-function if kept.
- **Stringly-typed `Settlement.time`/`timezone`** parsed on every `at_date` call. Parse once at the boundary: store `jiff::civil::Time` (custom serde "HH:MM") and validate the IANA zone at deserialization. This is the doc's own "parse, don't validate" philosophy (§2.5) applied inconsistently.
- **Three copies of month/leap-year arithmetic**: `add_months`/`days_in_month` duplicated in `cashflows/mod.rs` and `pricing/bond.rs`, plus a third `is_leap_year` in `daycount.rs`. Consolidate into `dates` (e.g. `Date::add_months`) — or just delegate to jiff, whose `Span` month arithmetic already does end-of-month constraining.
- **`HashMap<String, bool>` as a set** (`config.rs:75`) → `HashSet<String>`.
- **Linear pillar scan in `DiscountCurve::rt`** → `partition_point` (binary search); also `Calendar::count_business_days_fractional` walks day-by-day (O(days) — ~11k iterations for a 30y tenor). Neither is hot yet; fix the first (it's also *more* readable), note the second.
- **`Date::act365_year_fraction`** duplicates `DayCount::Act365Fixed` — remove or delegate.
- **`Error` is all `String` payloads** — accepted by design, fine; but a `#[from] jiff::Error` variant would remove a dozen `map_err(|e| Error::Date(e.to_string()))` sites.
- **`EuropeanOption` contains `exercise_style: ExerciseStyle` with an `American` variant** — the type name lies, the constructor pins `European`, but deserialization happily accepts `"American"`. Rename to `VanillaOption` (build-plan vocabulary) or delete the field until American exercise is priceable.
- **`tick_size` is stored and never used.** Either quantize marks/P&L to tick (nice: kills f64→Decimal dust in reports) or drop it.
- **CLI dependencies leak into the library.** `clap` (and arguably `toml`, `config`/`reports`/`portfolio`) are application concerns inside a published crates.io library. Feature-gate: `[features] cli = ["dep:clap"]` + `required-features = ["cli"]` on the bin; consider an `app` feature for config/reports if you want the core instrument/pricing crate minimal.

### Robustness

Strong: market-data `merge` rejects duplicates; config cross-checks deals↔instruments and warns on missing curves/spots; settlement time validation (the uncommitted change) closes the 25:99 hole; curve constructor validates sortedness/non-emptiness. Gaps are B3–B6 above plus: `DiscountCurve::new` doesn't reject non-finite rates or pillars at/before `base_date` (dates before base silently get df = 1.0 via the `t <= 0` early return); `price_future`'s `Err(_) =>` fallback silently degrades a misconfigured settlement to date-level expiry — consider logging/warning once at load (config already validates instruments; add `settlement.at_date` to the checks).

### Simplicity

The codebase is small (~3.4k lines of library code), readable in one sitting, and has essentially no over-abstraction — the complexity budget is spent on the right things (dates, conventions). The main simplicity debt is D2: a parallel schedule implementation *plus* an unused one is more total complexity than one shared mechanism.

## 4. Improvement Plan

### Phase A — Correctness (small diffs, do immediately, ~1 session)

> **Status: completed 2026-06-12**, qloxide commits `6498cde..aec9911`. A3 resolved by removing the Act252 variant; A2 is a JSON format break (`"frequency": 2` → `"frequency": "SemiAnnual"`).

| # | Fix | Files |
|---|-----|-------|
| A1 | B1: drop `* sign.signum()` in Volatility branch; add reversed-direction regression test | `dates/calendar.rs` |
| A2 | B4: `Bond.frequency: u32` → `cashflows::Frequency` (JSON break, pre-1.0 OK); delete `12 / freq` | `instruments/bond.rs`, `pricing/bond.rs`, fixtures |
| A3 | B2: decide Act252 — recommend removing the variant until a real `Bus252 { calendar }` is needed; second-best: doc-comment the calendar-day deviation prominently | `dates/daycount.rs` |
| A4 | B3: make `valuate` total — unpriced deals appear in output with an error reason; pnl report prints them | `portfolio.rs`, `reports.rs` |
| A5 | B7: reports use `inst.maturity()` + `Date` ordering (already planned in options-plan Step 8 — pull it forward) | `reports.rs` |
| A6 | B6: dispatch array-vs-object on first non-WS byte | `config.rs` |
| A7 | B5: remove `spot_date` default impl from `PricingContext` (or require explicit) | `pricing/mod.rs` |
| A8 | Clippy clean (4 warnings), remove crate-level `too_many_arguments` allow | misc |

### Phase B — Structural alignment (1–2 sessions, before options work grows the surface)

> **Status: completed 2026-06-12**, qloxide commits `51cba45..06d3244`. Notes: B-2 added `DayCount::accrue` (Decimal accrual, division last) so contractual amounts stay exact; bonds with odd tenors now have a short *first* stub. B-6 resolved by deriving `pay_date()`. B-3 keeps `Settlement::new(&str)` ergonomics (panics on bad literals, per the `Date::new` convention) while JSON deserialization validates time and timezone.

| # | Change | Rationale |
|---|--------|-----------|
| B-1 | Consolidate month/leap arithmetic into `dates` (or delegate to jiff `Span`); delete the two duplicates | three copies today |
| B-2 | Wire cash-flow-first: `Bond::cash_flows(&self) -> Vec<CashFlow>` built on `CashFlowSchedule`; `price_bond` becomes Σ amount·df. One stub convention everywhere. (Trait method can wait; an inherent method is enough.) | D2 — makes the architecture true |
| B-3 | `Settlement.time` → `jiff::civil::Time` with "HH:MM" serde; validate timezone at deserialize | parse-don't-validate |
| B-4 | Introduce `Position { instrument_id, side, quantity, avg_price }`; `compress` returns it; document the VWAP-includes-realized choice or switch to remaining-lot basis | D3 |
| B-5 | Currency consistency check at config load (same id ⇒ identical conventions, else warning) | D4 |
| B-6 | Resolve `pay_date` stored-derived-field question (derive or validate) | D5 |
| B-7 | Feature-gate CLI (`cli` feature, `required-features` on bin) | published-crate hygiene |
| B-8 | Update `qloxide-architecture.md`: rewrite §9 to the options-plan interface, record Position/VWAP decisions, mark Volatility calendar as implemented | D1; docs must match code |
| B-9 | Commit the pending work — the qloxide working tree changes (venue removal, settlement validation, day-count tests) are coherent and test-green; parent repo: verify the `QuantMath` submodule pointer change is intentional before committing | hygiene |

### Phase C — Thereafter (the existing roadmap, unchanged in substance)

1. **Options Phase 1** per `docs/options-plan.md`: Step 0 rename (`spot`→`market_price`, `spot_date`→`valuation_date`) — do this *after* Phase A/B to avoid rebasing fixes across the rename — then libm, `math` module, Black76 + Greeks, `VolSurface::Flat`, european pricer, contract-size P&L, Brent option example. The plan is detailed, has reference values from `docs/opt_src/gbsm.rs`, and needs no rework.
2. **Options Phase 2/3**: implied vol (Brent root-finder), Bachelier (`FlatBachelier` surface variant).
3. **Layer 6**: commodity forward curves from futures strips (prerequisite for Asian options and for pricing unlisted tenors).
4. **Swap pricer** (deterministic, needs RateIndex resolution + forward projection) — currently the largest *defined-but-unpriced* instrument; schedule after forward curves or before, depending on whether rates or commodities lead.
5. **Scenario engine** (options-plan Phase 5) — the uniform `price(inst, ctx)` design already supports it; mostly a `PricingContext` transformer.
6. **Cash account / settlement ledger** (architecture doc §14) — reuses `CashFlow`; unblocked once B-2 makes instruments actually emit flows.
7. **CI**: builds.sr.ht manifest running `cargo test` + `cargo clippy -- -D warnings` + `cargo fmt --check`.

## 5. Verdict

The design is thoughtful and the documentation of decisions is well above typical hobby-project standard; the typetag/reference-data/three-time-types choices are correct for the stated domain. The implementation is clean, idiomatic, and well-tested *for the paths it exercises* — but it has one verified logic bug (B1), one wrong-by-definition convention (B2), and one dangerous silent-omission path in P&L (B3) that should be fixed before any new pricing capability is added. The single biggest architectural gap is that the library's stated core abstraction (cash flows) is not yet load-bearing (D2). Phases A+B are roughly 2–3 focused sessions and leave a substantially more trustworthy base for the options work, which is already well-planned and should proceed unchanged afterwards.

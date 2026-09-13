# qloxide: Build Status & Mono-Repo Notes

> **Design decisions moved:** the architecture rationale (time types,
> cash-flow-first, reference data, typetag polymorphism, curves, pricing
> interface, QuantMath comparison) now lives in `qloxide/ARCHITECTURE.md`,
> shipped with the crate. This file keeps what is mono-repo-specific:
> build progress, file locations, repo/publishing logistics, and open items.
> See also `docs/qloxide-review-2026-06-12.md` for the code review and
> improvement phases (A and B completed).

## 1. Build Order

```
Layer 0: Foundation
  - Error enum (thiserror, all variants upfront)
  - Date newtype (jiff::civil::Date, custom serde "YYYY-MM-DD")
  - Timestamp newtype (jiff::Timestamp)
  - Zoned newtype (jiff::Zoned)

Layer 1: Date Logic
  - Calendar (holiday schedules, business day counting)
  - DateRule (business day adjustment rules)
  - DayCount enum (Act360, Act365Fixed, Thirty360, ActActIsda; Act252 removed in 0.2.0)
  - Compounding enum (Continuous, Simple, Annual, SemiAnnual, Daily)

Layer 2: Reference Data
  - Currency { id, settlement, day_count }
  - CreditEntity { id, currency }
  - RateIndex { id, currency, tenor, day_count, compounding, fixing_source }

Layer 3: Cash Flows
  - CashFlow { amount, currency, pay_date }
  - CashFlowSchedule (date generation for swap legs etc.)

Layer 4: Discount Curves
  - DiscountCurve (parameterized by DayCount, linear interpolation)
  - Rate conversions between compounding conventions

Layer 5: Instruments (typetag trait)
  - FinancialInstrument trait definition (6 methods: id, currency, settlement, maturity, instrument_type, as_any)
  - Swap (fixed/float legs, referencing CashFlowSchedule)
  - Future (venue, settlement session, expiry)
  - EuropeanOption (strike, put/call, exercise style)
  - Bond, Equity, FxForward, Basket

Layer 5.5: Trades
  - Deal { instrument_id, direction: BuySell, quantity: Decimal, price: Decimal, timestamp, counterparty }
  - BuySell enum, signed_quantity() method; Position (compression)

Layer 6: Commodity Forward Curves (not started)
  - Forward term structure from futures prices

Layer 7: Pricing (partial — see Pricing Build Order below)
  - PricingContext trait (market data: prices, curves; vol surfaces later)
  - Pricing functions per instrument type (dispatch via as_any() downcast)
  - portfolio::valuate — per-deal MTM and P&L

Note: Decimal migration complete — CashFlow.amount, Deal.price, Deal.quantity all use rust_decimal::Decimal.
```

Each layer depends only on layers below it. Each step adds a cohesive group of types, compiles, tests JSON roundtrip, and pauses for discussion.

---

## 2. Pricing Build Order


| Step | What | Status |
|------|------|--------|
| 1 | PricingContext trait + MarketData impl | Done |
| 2 | `price_bond` — discounts `Bond::cash_flows()` | Done |
| 3 | `price_future` — mark / settlement price, timestamp-aware expiry | Done |
| 4 | `portfolio::valuate` — per-deal MTM and P&L | Done |
| 5 | Vol surface + Black76 + `price_european` | Planned — `docs/options-plan.md` Phase 1 |
| 6 | Forward curves (Layer 6) — commodity term structure | To do |
| 7 | `price_swap` — fixed leg PV vs floating leg PV (needs RateIndex resolution) | To do |
| 8 | Bachelier (`FlatBachelier` surface variant) | Planned — options plan Phase 3 |
| 9 | Monte Carlo — path-dependent exotics | Later |

---

## 3. File Locations

Mono-repo root: `~/dev/sourcehut/ql/`

| Path | Contents |
|------|----------|
| `qloxide/` | The library (standalone git repo, gitignored here; Layers 0–5.5 + partial pricing) |
| `qloxide/ARCHITECTURE.md` | Design decisions (moved from this document) |
| `QuantMath-test-rs/` | Reference reimplementation (86 tests, pricing-only) |
| `QuantMath/` | Original QuantMath (read-only submodule, pinned to pristine upstream) |
| `docs/qloxide-build-plan.md` | Original build plan (superseded) |
| `docs/options-plan.md` | Options pricing plan (Phase 1: Black76) |
| `docs/qloxide-review-2026-06-12.md` | Code review; improvement phases A and B completed |
| `docs/qloxide-architecture.md` | This document |

## 4. Repository & Publishing

### 4.1 One public repo (since 2026-09-13)

| Aspect | Detail |
|--------|--------|
| **Repository** | `https://github.com/theHausdorffMetric/qloxide` — development and releases in one repo, branch `master` |
| **Local clones** | `~/dev/ideas/sourcehut/qloxide/` (submodule of the `ideas` monorepo, workstation) and Dan's clones on Tuxedo |
| **Who pushes** | the agent from the workstation (fine-grained token scoped to this repo, contents only) and Dan |
| **Publishing** | crates.io (`qloxide`); `qloxide-analytics` (`analytics/`) is a workspace member and is not published |
| **Release** | Dan, on Tuxedo: review `git diff v<prev>..master`, `just ci`, `cargo publish --no-verify`; the published commit is then tagged `v<version>` |
| **Guards** | GitHub ruleset on `master` (no force-push, no deletion); publishing is never automated, so a tag alone publishes nothing |

### 4.2 History

Until 2026-09-13 the code lived on sourcehut in two repos — a private dev
fork under `~dpclaude` and a public release repo under `~danprobst`, fed by
patches — and the crates.io metadata pointed at the latter. sourcehut's
terms of service ban LLM-assisted content from 2026-09-10, so both repos
were retired and the full dev history (Claude co-author trailers included)
moved to GitHub. The release copy had been a plain mirror of the dev
history, so nothing was lost: releases 0.1.0–0.5.0 were cut from that
setup, and `v0.5.0` on GitHub is the commit crates.io 0.5.0 was built from.

## 5. Open Items

| Item | Status | Notes |
|------|--------|-------|
| Repo hosting | Done | GitHub `theHausdorffMetric/qloxide` since 2026-09-13 (the two sr.ht repos are retired, §4.2) |
| Settlement session modeling details | Done | Settlement struct: venue + session + time + timezone |
| Step-by-step implementation plan | Done | All original steps complete or superseded |
| crates.io name reservation | Done | Published as `qloxide` on crates.io |
| Commodity forward curves | To do | Layer 6 — term structure from futures prices |
| Pricing & risk | Partial | Uniform `price(instrument, context)`, separate module — see `qloxide/ARCHITECTURE.md` §8; Greeks/scenarios to come with options |
| Decimal migration | Done | CashFlow, Deal price/qty use `rust_decimal::Decimal` |
| Cash account | To do | Track variation margin, final settlement, coupons, option premiums. Reuse `CashFlow` type. Introduces time-series state (daily settlement engine). Prerequisite: forward curves + basic pricing more complete. Enables funding cost tracking, realized P&L ledger, margin reconciliation. |

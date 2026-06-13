# Changelog

All notable changes to qloxide are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/) (pre-1.0: breaking changes bump
the minor version).

## [Unreleased]

### Breaking

- Renamed across the API and JSON formats: `spot` → `market_price`,
  `spot_date` → `valuation_date` ("spot" means prompt physical delivery
  in commodities; "spot date" means T+2 in FX). **JSON migration:** in
  market data files, `"spots"` → `"market_prices"` and `"spot_date"` →
  `"valuation_date"`. `settlement_price` is unchanged.

### Added

- **European options on futures price with Black76** (Phase 1 of the
  options effort): `math` module (`norm_pdf`/`norm_cdf` via libm),
  `pricing::black76` (price + delta/gamma/vega/theta, validated against
  reference values and finite differences), `VolSurface` (`Flat` variant;
  the surface variant carries the model), `vol_surfaces` in market data
  JSON, and `pricing::european` wired into the uniform
  `price(instrument, context)` dispatch. American exercise is rejected
  explicitly. Expired options return intrinsic against the underlying's
  settlement price.
- Option deal P&L is in dollar terms via the underlying future's
  contract size.
- Config checks validate option pricing inputs (underlying exists, vol
  surface present); the instruments report shows model prices (labelled
  `model`) for instruments without a market quote.
- The Brent example includes a call option (`ICE-BRN-K26-C-75`).
- `ARCHITECTURE.md` — design decisions, reasoning, and rejected
  alternatives, shipped with the crate (migrated from the development
  mono-repo).

## [0.2.0] — 2026-06-12

### License

Relicensed from MIT to **GPL-3.0-or-later**. Versions up to 0.1.0 remain
available under MIT on crates.io; that grant is irrevocable for those
versions.

### Breaking

- `Bond.frequency` is now the `cashflows::Frequency` enum instead of a raw
  `u32`. **JSON migration:** `"frequency": 2` → `"frequency": "SemiAnnual"`
  (`1` → `"Annual"`, `4` → `"Quarterly"`, `12` → `"Monthly"`). The old
  `u32` allowed `0` (panic in the pricer) and silently mispriced values
  that don't divide 12.
- `Settlement.time` is a typed minute-precision `dates::Time` instead of a
  string. The JSON format is unchanged (`"19:30"`), but invalid times and
  unknown IANA timezones are now rejected at deserialization instead of
  failing at first use. `Settlement::new` panics on invalid literals.
- `EuropeanOption.pay_date` is no longer a stored field; use the
  `pay_date()` method, which derives it from `settlement.payment_lag`
  applied to expiry. Old JSON containing `pay_date` still loads (the field
  is ignored).
- `Deal.venue` removed — venue is instrument-level (on `Settlement`), not
  deal-level. `counterparty` remains.
- `DayCount::Act252` removed: it divided **calendar** days by 252, but the
  BRL convention counts **business** days and needs a holiday calendar. A
  correct `Bus252 { calendar }` will be added when needed.
- `portfolio::valuate` now returns one `ValuedDeal` per input deal, each
  carrying `Result<Valuation>` — deals that cannot be priced are reported
  with the reason instead of being silently dropped. The `pnl` report
  prints `UNPRICED` rows and a warning that they are excluded from totals.
- `portfolio::compress` returns the new `Position` type
  (`instrument_id`, `direction`, `quantity`, `avg_price`) instead of
  synthetic `Deal`s. Note: `avg_price` is net cost / net quantity, which
  embeds realized P&L of closed lots — not the FIFO basis of remaining lots.
- `PricingContext::spot_date` no longer has a default implementation
  (the default derived the date from `as_of()` in UTC, rolling evening
  snapshots onto the next day). Implementors must provide it explicitly.

### Added

- `Bond::cash_flows()` — bonds decompose into contractual cash flows
  (coupons + principal) via `CashFlowSchedule` (backward generation, short
  first stub); `price_bond` discounts these flows.
- `DayCount::accrue(base, from, to)` — accrual computed entirely in
  `Decimal` with the division last, so decimal-exact contractual amounts
  (e.g. 30/360 coupons) carry no f64 noise.
- `dates::Time` — minute-precision time-of-day newtype, serialized
  `"HH:MM"`.
- `Date::add_months` — month arithmetic with end-of-month clamping
  (delegates to jiff), replacing duplicated implementations.
- `Frequency::per_year()`.
- Config load warns when two instruments define the same currency id with
  different conventions (day count / settlement rule).
- `deny.toml` — `cargo deny check` gates RUSTSEC advisories, license
  compatibility, duplicate versions, and crates.io-only sources.
- `cli` cargo feature (default on): library consumers can drop the clap
  dependency with `default-features = false`.

### Fixed

- `Calendar::Volatility::count_business_days` returned the wrong sign when
  counting backwards (`from > to`).
- Reports derive expiry and expired-status from `instrument.maturity()`
  instead of a `Future`-only downcast with string date comparison, so
  bonds and options are handled correctly.
- Config files containing a JSON array with a malformed element now report
  the element's actual error instead of a misleading "expected a single
  object" failure.
- Settlement times like `"25:30"` are rejected with a clear error.

### Changed

- `toml` dependency upgraded to 1.x; lockfile refreshed (clears the
  RUSTSEC-2026-0097 advisory on the never-compiled optional `rand`
  dependency).
- Bonds whose tenor is not a whole number of coupon periods now have a
  short **first** stub (backward schedule generation) instead of a short
  final stub.

## [0.1.0] — 2026-02-08

Initial release (MIT): layers 0–5.5 — error types; `Date`, `Timestamp`,
`Zoned` (jiff newtypes); calendars, date rules, day counts, compounding;
reference data (`Currency`, `CreditEntity`, `RateIndex`); `CashFlow` and
`CashFlowSchedule`; `DiscountCurve`; typetag-based `FinancialInstrument`
trait with `Bond`, `Equity`, `EuropeanOption`, `Future`, `FxForward`,
`Swap`, `Basket`; deals and blotter compression; deterministic pricing for
bonds and futures; TOML/JSON config loader, CLI binary, and portfolio
reports (instruments, deals, positions, pnl).

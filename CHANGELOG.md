# Changelog

All notable changes to qloxide are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/) (pre-1.0: breaking changes bump
the minor version).

## [Unreleased]

### Breaking

- `Future` and `EuropeanOption` gain a required `clearing` field
  (`"ICE" | "CME" | "bilateral"`, new `instruments::Clearing` enum) — the
  intrinsic fact of where a derivative clears, distinct from the `Settlement`
  fixing conventions and from marking policy. Required with no default:
  every instrument states its nature explicitly. Existing `instruments.json`
  files must add the field; constructors take one more argument.

### Breaking (report taxonomy)

- The **instruments report is now a pure specification listing** (ID, type,
  underlying, currency, clearing, put/call, strike, expiry) — no prices, no
  status column. Valuation lives in `pnl` alone; the old universe-mark-sheet
  behavior is retired (a `risk` report will carry model diagnostics later).
- **`market_data` is optional in the config**: static reports (instruments,
  deals, positions) run without it. `Portfolio::market_data` is now
  `Option<MarketData>`; `reports::pnl` returns `Result<String>` and errors
  without market data; market-dependent load-time checks are skipped when
  absent.

### Added

- `MarketStore::day` cross-checks settle coverage: the loaded file's
  `settlement_prices` keys must match the manifest day's `settles` list
  exactly, so a drifted manifest can no longer pass the O(manifest)
  completeness gate while the day file disagrees. The error names the
  settles missing from the file and the ones the manifest omits.
- `MarketData` file-level provenance stamps: optional `source` (e.g. `"ICE"`;
  scenario overlays self-declare `"scenario:…"`) and `generator` fields with
  accessors/setters; `merge` keeps this side's stamps, filling from the other
  only if absent.
- `settlement_prices` semantics generalized: the official settle at the
  valuation date for any listed instrument (options included), not only the
  frozen final settle of expired ones — groundwork for settle-primary
  marking.
- `pricing::decimal_to_f64` is now public, so market-data drivers can match
  instrument Decimals against f64 quotes under the pricers' own conversion.
- **Settle-primary official marks** (deal valuation): `portfolio::valuate`
  now marks cleared instruments directly from their settlement price —
  missing settle = per-deal error, never a silent model fallback; bilateral
  instruments (and types without a clearing dimension) mark to model. New
  `portfolio::MarkSource` (`Settle`/`Model`) on `Valuation`, surfaced as a
  **Source column** in the pnl report. `FinancialInstrument` gains a
  `clearing()` accessor (`None` default; overridden by `Future` and
  `EuropeanOption`). Config validation warns on a cleared instrument
  without a settlement price. P&L figures print with two decimals.

- New example `examples/brent-timespread/`: long Jul/Dec Brent time
  spread struck 2026-02-02 at that day's settles (+1.29 backwardation).
  The Jul leg expires 2026-05-29 mid-history — `pnl-series` shows the
  P&L recomposing to realized at the frozen final settle and the book
  degrading into an outright Dec short; the risk report shows the frozen
  leg excluded from greeks (this example caught that fix: expired
  positions carry no market risk and must not net against live exposure;
  model value now covers live positions only, keeping scenario diffs
  pure).
- **`risk` report** — the model world in one place: position-scaled
  Black-76 greeks with portfolio totals (futures delta 1; expired/unknown
  types listed as skipped), the **calibration diagnostic** (model vs settle
  per cleared live option, `!`-flagged beyond a cent — the old acceptance
  test as a monitored check), model marks for anything without an official
  settle, and the **model value of the book** (the number scenario runs
  diff, since official settle-primary marks by design do not respond to
  bumped inputs).
- **`--market <file>` CLI override** (`config::load_with_market`): replace
  the config's `market_data` with one explicit file — the scenario entry
  point for pricing a book against a bumped market without editing the
  config.
- **`pnl-series` report** — daily portfolio P&L trajectory over the market
  series, *composition-aware*: each trading day in [earliest deal date,
  evaluation date] values the book as it existed that day (a deal
  contributes from its inception date), with realized/unrealized/total and
  the daily change (variation-margin view — day changes telescope to the
  final total). Expiry cash-settlement lands as realized P&L at the frozen
  final settle. Requires `market_series`; refuses on integrity errors.
  Valuing today's *full* book against a historical day file remains
  available as an explicit what-if, and is now documented as such.
  New `portfolio::valuate_at(deals, instruments, market_data)` values
  deals against an explicit snapshot (the per-day entry point).
- **`market_data::MarketStore`** — manifest-driven historical series
  (date → snapshot): loads a generator-written `manifest.json` (per-day
  files, settle-coverage lists, skipped non-trading days, provenance
  stamps), answers calendar/coverage queries in O(manifest), and lazily
  loads + validates individual day snapshots.
- **Series settle-completeness at load**: new optional `market_series`
  config key (manifest path) and `[[waivers]]` (date-scoped, with reason).
  For every cleared, dealt instrument the loader walks
  [earliest deal date, evaluation date]: trading days without a settle and
  calendar days the series doesn't account for become **integrity errors**
  (collapsed into contiguous runs); findings inside a waiver window
  downgrade to warnings. `Portfolio` gains `market_series` and
  `integrity_errors`; pnl refuses while integrity errors are present,
  static listings still render. `MarketData::settlement_ids()` added for
  generators writing coverage.

### Docs

- Imported design, planning, and review notes under `docs/` (options plan,
  code review, build plan/status, upstream-QuantMath reference notes, and the
  `opt_src/` reference pricing implementations), carried over from the retired
  `ql` scaffold mono-repo. Added a `docs/` index and surfaced the upstream
  [QuantMath](https://github.com/MarcusRainbow/QuantMath.git) repo link in
  `README.md` and `ARCHITECTURE.md`.

## [0.3.0] — 2026-06-13

### License

Relicensed from MIT to **GPL-3.0-or-later**. Versions up to and including
0.2.0 remain available under MIT on crates.io; that grant is irrevocable for
those versions.

### Breaking

- Renamed across the API and JSON formats: `spot` → `market_price`,
  `spot_date` → `valuation_date` ("spot" means prompt physical delivery
  in commodities; "spot date" means T+2 in FX). **JSON migration:** in
  market data files, `"spots"` → `"market_prices"` and `"spot_date"` →
  `"valuation_date"`. `settlement_price` is unchanged.
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

## [0.2.0] — 2026-03-11

Feature release, published under MIT.

### Added

- `Decimal` migration — cash flow amounts, trade prices, and quantities use
  `rust_decimal::Decimal` for exact representation; added the trades layer
  (`Deal`, `BuySell`) and the market data and pricing modules.
- `DiscountCurve` and `PricingContext` prepared for multithreading.
- `Settlement` wired for timestamp-level pricing, with a settlement payment
  lag.
- TOML/JSON config loader, the `qloxide` CLI binary, and portfolio reports
  (instruments, deals, positions, pnl).

### Changed

- `CLAUDE.md` excluded from the published package.

## [0.1.0] — 2026-02-08

Initial release (MIT): layers 0–5.5 — error types; `Date`, `Timestamp`,
`Zoned` (jiff newtypes); calendars, date rules, day counts, compounding;
reference data (`Currency`, `CreditEntity`, `RateIndex`); `CashFlow` and
`CashFlowSchedule`; `DiscountCurve`; typetag-based `FinancialInstrument`
trait with `Bond`, `Equity`, `EuropeanOption`, `Future`, `FxForward`,
`Swap`, `Basket`; deals and blotter compression; deterministic pricing for
bonds and futures; TOML/JSON config loader, CLI binary, and portfolio
reports (instruments, deals, positions, pnl).

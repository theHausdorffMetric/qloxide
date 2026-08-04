# Design note: the settlement-index registry & uniform futures

*2026-08-04 · v3 for the sr.ht qloxide repo · supersedes v2 (2026-08-03)*

*v3: reshaped after the futures-redesign discussion (step-by-step
walk-through of the ICE Naphtha CIF NWE Cargoes future) and the decision
that **breaking changes are in scope** at the current level. The v2
`SettlementTerms` enum on the instrument is gone: `Future.underlying`
becomes a required reference into a settlement-index registry, windows
moved fully into the index graph as per-period `Average` entries, and
settlement-at-termination gained a target type (`Payment`, shipped on the
`payment-instrument` branch). The v2 research background is unchanged and
still governs:
[`settlement-index-research-quantmath.md`](settlement-index-research-quantmath.md),
[`settlement-index-research-industry.md`](settlement-index-research-industry.md).*

## Problem

Unchanged from v2, plus one requirement sharpened by the discussion:

- `Future.underlying` is free text used as a display column; the
  economically defining fact — **what the contract settles against** — is
  not representable. The Brent APO work carried it as registry comments
  and hand-applied corrections.
- **Uniformity requirement (new in v3):** a Brent future, a Brent 1st
  Line future, and a naphtha cargoes future are *legally identical
  objects* — exchange contracts with daily prints, one final published
  settlement value, and one cash settlement date. Their differences are
  entirely in *how the final number is generated*. The instrument struct
  must therefore be **the same for all of them**; everything
  generation-related belongs in reference data.

## Doctrine (kept from v2)

The resolve-boundary architecture survives verbatim; only the data layout
changed. One booked instrument, always — the averaging-swap view is
derived structure, produced per pricing request, never stored:

1. **Resolve** — index graph + contract expiry → dated observations, each
   mapped to a concrete contract (a September determination month maps to
   two Brent contracts across the roll).
2. **Partition** — per observation against the valuation date, in the
   market-data layer: published fixing (hard error if a past one is
   missing) vs forward off the curve. Realized fixings are constants, so
   they freeze under bumps and delta migrates off the contract with zero
   bespoke risk code.
3. **Overlay** — any residual gap between the margined print and the
   decomposed expectation is a constant basis/convexity adjustment
   (`RequiredOnlyForValuation`), never a representation change.

**Descent criterion** (unchanged): decompose exactly when the series'
constituents are inside the modeled risk universe *and* the request needs
their sensitivities or joint law. BRN's own index (BFOE cash market)
stays an opaque leaf; BFL's constituents are the Brent curve itself.

The consumer ladder over the single instrument:

| Consumer | Reads | Needs |
|---|---|---|
| Margin / EOD P&L | settle print by contract id | nothing new |
| Theoretical PV, missing marks | resolve + partition | fixing store + forward curve |
| Curve risk / scenarios | same, under a shifted context | nothing further — freezing is emergent |
| APO pricing | full decomposition + vol surface | T-W + seasoned-strike rewrite |

The naphtha contract shows the ladder is not an abstraction: ICE's own
daily settle during the running month **is** the decomposition formula —
`(m/n)·Ā_realized + ((n−m)/n)·F_bom` — evaluated with ICE's
balance-of-month estimate. Level-2 valuation is the identical expression
with *our* balmo curve in the `F_bom` slot; the last print converges to
the realized monthly average, which is why level 1 suffices for P&L.

## Design

### `Future` — uniform across all products

```rust
pub struct Future {
    pub id: String,
    pub underlying: IndexRef,     // REQUIRED FK into the settlement-index registry
    pub currency: Arc<Currency>,
    pub settlement: Settlement,   // procedure: venue/session/time/tz/payment_lag — unchanged
    pub clearing: ClearingStatus,
    pub expiry: Date,             // LTD — the ONLY date the contract owns
    pub contract_size: Decimal,
    pub tick_size: Decimal,
}
```

- `underlying` changes meaning from prose to key (**breaking**). Display
  labels are derived: `registry[underlying].display_name`.
- There is **no window field and no `SettlementTerms`**. The governing
  invariant: **a contract contributes exactly one date — its expiry.
  Every other date in the system lives in the index graph.**
- `settlement` already expresses the full settlement *procedure*,
  including "cash two clearing-house business days after LTD"
  (`payment_lag: DateRule::BusinessDays`). The procedure is uniform
  machinery and was never the product differentiator.

### The settlement-index registry

```rust
pub struct SettlementIndex {
    pub id: IndexRef,             // newtype over String
    pub display_name: String,     // retires the free-text label role
    pub rule: IndexRule,
    pub calendar: String,         // publication calendar NAME (no fincal dep)
}

pub enum IndexRule {
    Published { source: String },                     // opaque leaf: Platts assessment, ICE Brent Index
    FrontLine { of: IndexRef, roll: RollRule },       // derived front-month selector
    Average   { of: IndexRef, window: (Date, Date) }, // concrete determination period, explicit dates
    Spread    { legs: Vec<(Decimal, IndexRef)> },     // weighted legs; ±1 until cracks need real weights
}

pub enum RollRule { RollOnExpiry }                    // single-variant v1
```

The registry is a graph (recursion is a feature) with **two tiers**:

- **Curated structural nodes** — month-free, hand-maintained: `Published`
  leaves, `FrontLine` selectors, `Spread` combinations.
- **Generated period nodes** — `Average` entries materialized by the
  driver at listing time, exactly like expiries are. The driver knows the
  strip and the publication calendar; qloxide reads explicit dates and
  needs **no calendar math to construct windows**. Balmo is not a special
  case — just an `Average` with different dates.

A period-average index is legitimate *shared* reference data: the future,
balmo/monthly swaps, and APOs all settle on the same period object, and
two venues' lookalikes share one (proxy identity becomes checkable). What
is deliberately **not** in the registry: per-contract prints (market
data, keyed by contract id) and fixings (stored under the *leaf* series,
one print per publication day — period indices are computed, never
fixed).

Worked example (naphtha + Brent side by side):

```json
// curated tier
{ "id": "PLATTS-NAPHTHA-CIF-NWE", "display_name": "Naphtha CIF NWE cargoes (Platts)",
  "rule": { "Published": { "source": "PLATTS" } }, "calendar": "PLATTS-EUR" }
{ "id": "ICE-BRENT-INDEX", "display_name": "ICE Brent Index",
  "rule": { "Published": { "source": "ICE" } }, "calendar": "ICEUK" }
{ "id": "ICE-BRENT-1L", "display_name": "Brent 1st Line",
  "rule": { "FrontLine": { "of": "ICE-BRN", "roll": "RollOnExpiry" } }, "calendar": "ICEUK" }

// generated tier (driver, at listing)
{ "id": "PLATTS-NAPHTHA-AVG-2026-09",
  "rule": { "Average": { "of": "PLATTS-NAPHTHA-CIF-NWE", "window": ["2026-09-01","2026-09-30"] } } }
{ "id": "PLATTS-NAPHTHA-BALMO-2026-08-05",
  "rule": { "Average": { "of": "PLATTS-NAPHTHA-CIF-NWE", "window": ["2026-08-05","2026-08-31"] } } }
{ "id": "ICE-BRENT-1L-AVG-2026-09",
  "rule": { "Average": { "of": "ICE-BRENT-1L", "window": ["2026-09-01","2026-09-30"] } } }

// futures — structurally identical; only id / underlying / expiry vary
{ "type": "Future", "id": "ICE-NAF-U26", "underlying": "PLATTS-NAPHTHA-AVG-2026-09", "expiry": "2026-09-30", ... }
{ "type": "Future", "id": "ICE-BFL-U26", "underlying": "ICE-BRENT-1L-AVG-2026-09",   "expiry": "2026-09-30", ... }
{ "type": "Future", "id": "ICE-BRN-U26", "underlying": "ICE-BRENT-INDEX",            "expiry": "2026-07-31", ... }
```

### Evaluation semantics

- **Terminal** (underlying is `Published`/`FrontLine`/`Spread`): the
  settle value is the index value at the contract's expiry, adjusted by
  the index calendar. Terminal indices stay *functions* — no per-month
  terminal entries, because the evaluation date is the one date the
  contract already carries.
- **Average**: the settle value is the mean of the `of`-series over the
  entry's explicit window. Period indices are *materialized*.

This asymmetry is chosen, not accidental — it is exactly the
one-date-per-contract invariant applied twice.

### Validation (load-time, hard — breaking changes accepted)

- Every `Future.underlying` must resolve in the registry. **No
  opaque-leaf-by-absence**: opacity is declared *in* the registry
  (`Published` — defined, constituents unmodeled), a dangling ref is an
  error, full stop.
- Dangling `of`/`legs` refs and reference cycles: error (DFS, one pass).
- For an `Average`-underlying future: `window.end == expiry` (both are
  the last working day of the determination month for these products).
  This catches "contract points at the wrong month's index" — a bug
  class v2 could not even express a check for.
- Serialization stays embed-by-value like `Currency`, with the existing
  load-time consistency check extended to index ids.
- Quick win, shippable independently: dangling `option.underlying`
  silently defaults `contract_size = 1` (`portfolio.rs`) — warn now,
  error later.

### Settlement is a conversion (`Payment`)

`Payment { id, credit_id, amount, currency, settlement, pay_date }` is a
bookable instrument (typetag-registered; PV = discounted amount; settled
payments price at zero — implemented on the `payment-instrument` branch).
Termination becomes expressible in the instrument algebra instead of
falling off its edge:

- cash-settled future at expiry → one `Payment`
  (`(final index value − price) × size` at `expiry + payment_lag`);
- physical delivery, when needed → deliverable position + `Payment`
  (a statement about what the contract *converts into*, not an
  `IndexRule` variant);
- `FxForward` is re-foundable as exactly two `Payment`s (it already
  carries the fields twice).

## What it buys

Everything from v2 (APOs structural, settlement verification generic,
risk freezing emergent, index algebra for diffs/cracks, proxy identity,
the observed smile relation) — plus:

- **Uniform futures**: one struct, one code path, no per-product
  variants; the BFL differs from naphtha by one registry node
  (`FrontLine` vs `Published` under the `Average`).
- **qloxide stays calendar-dumb for windows**: explicit, auditable dates
  in data; `CalendarSource` de-escalates to in-window day counting only.
- **Wrong-month binding is machine-checkable** (`window.end == expiry`).
- **Termination is closed** under the instrument algebra via `Payment`.

## Migration

Breaking changes are accepted at the current level (pre-1.0, all books
regenerable). No compat shims, no versioned format, no dual semantics:

1. Add `SettlementIndex` + registry loading + validation.
2. Flip `Future.underlying` to `IndexRef` semantics; **rewrite the
   example books** (`examples/*/instruments.json`) with real index ids.
3. Derive display labels from the registry; delete label usages of the
   old free string.
4. Drivers (qloxide-ice) emit the curated tier alongside their contract
   mappings and generate the period tier per listed strip.

## Decided / parked

Decided in v3:

- ~~Window location~~ — in the index graph as per-period `Average`
  entries (v1 had it on the index as a family, v2 on the instrument;
  v3 materializes it per period). The instrument carries no dates but
  expiry.
- ~~Migration policy~~ — hard referential integrity; breaking.
- ~~`RateIndex` relation~~ — separate namespaces; an `IndexRef` never
  names a `RateIndex`. State it in module docs.
- ~~Naming asymmetry~~ — `underlying` means "the thing one level down":
  instrument id on options, index id on futures. Accepted; document.
- ~~Cash representation~~ — `Payment` instrument (shipped).

Parked deliberately:

- **Physical delivery**: nothing in the current book needs it; design
  "position + Payment" when a product arrives, don't speculate.
- **Terminal publication lag** (Brent Index prints the day after
  expiry): use expiry adjusted by the index calendar until a real
  mispricing forces a `DateRule` on the index.
- **Convexity overlay**: defer until a consumer needs it (unchanged
  from v2).
- **`AsianOption`**: new instrument type (recommended — its pricer can
  structurally require an `Average` underlying) vs extending
  `EuropeanOption`; decide at M3.

## Slices

- **M1** — `SettlementIndex` types + registry + serde + embed/consistency
  + dangling-ref/cycle/window-expiry validation; `Future.underlying` →
  `IndexRef`; rewrite examples; `contract_size` warning quick win.
- **M2** — resolve + partition in the pricing module
  (`PricingContext::fixing(series_id, date)`, shared with swap floating
  legs); driver population (qloxide-ice curated tier + generated period
  tier; bitrepo trails).
- **M3** — `AsianOption` + Turnbull–Wakeman with the seasoned-strike
  rewrite K̂ = (nK − mĀ)/(n−m) (K̂ ≤ 0 ⇒ discounted cash + forward
  strip).

## References

- [`settlement-index-research-quantmath.md`](settlement-index-research-quantmath.md) —
  upstream QuantMath: `dependencies()`/`DependencyCollector`,
  `FixingTable`, `Instrument::fix` self-decomposition,
  `RequiredOnlyForValuation`, bump machinery with fixing freezing.
- [`settlement-index-research-industry.md`](settlement-index-research-industry.md) —
  Strata `OvernightFuture`/`ResolvedOvernightFuture`, QuantLib
  `OvernightIndexFuture` + Asian machinery, ICE BFL spec & 2012
  swaps-to-futures conversion, Endur/Murex practice, Henrard convexity,
  Hoogland & Neumann seasoned-strike identity.
- ICE Naphtha CIF NWE Cargoes Future (product 6753535) — the v3 worked
  case: LTD last working day of the month, cash two clearing-house
  business days later, final settle = monthly average of Platts daily
  publications, running-month prints = realized/balmo weighted average.
- bitrepo/icedat `docs/option_model_conventions.md` — the APO
  vol-convention section (T_eff fingerprint, SOFR discounting,
  flat-surface relation).
- qloxide-ice `src/registry.rs` — the `BFL` entry whose comment this
  proposal turns into data.
- dpdev `ql/brent-apo` — the worked case (real ICE marks via
  `qloxide-ice gen-series`).

# Design note: settlement series & terms as qloxide reference data

*2026-08-03 · draft v2 for the sr.ht qloxide repo · grew out of the Brent APO
work (dpdev `ql/brent-apo`, qloxide-ice `BFL` registry entry,
bitrepo/icedat `docs/option_model_conventions.md` §"Average price options")*

*v2 (same day): reshaped after design discussion + research. The window moved
from the index to the instrument (it is contract masterdata), roll moved into
the series definition, and the futures-vs-swap representation question is
resolved by the resolve-boundary architecture below. Research background:
[`settlement-index-research-quantmath.md`](settlement-index-research-quantmath.md)
(upstream QuantMath internals) and
[`settlement-index-research-industry.md`](settlement-index-research-industry.md)
(Strata/QuantLib overnight futures, ICE 2012 conversion, ETRM practice, APO
seasoning).*

## Problem

`Future.underlying` is an undocumented free-text `String`, used only as a
display column; `EuropeanOption.underlying` is the same-named field but is a
load-bearing foreign key (forward + vol lookup, contract-size inheritance,
generator planning). Meanwhile the *economically defining* fact about a
future — **what it settles against** — is not representable at all. The Brent
APO case showed the cost: "the 1st Line averages front-month ICE Brent settles
over the contract month, ICEUK publication days, roll-on-expiry" had to be
carried as a registry comment, a hand-applied Turnbull–Wakeman correction
(effective vol time T₁ + (T₂−T₁)/3), and per-case documentation. Every one of
those is derivable *if the instrument set knows its settlement index*.

## Two representations, one instrument: the resolve boundary

A BFL is *legally* a future (id, daily settle prints, cash settlement on a
date) and *economically* an averaging swap. That duality is historical fact,
not modeling artifact: ICE converted its cleared OTC oil swap complex to
futures in place on 2012-10-15 (Dodd-Frank). So when does a system switch
from the "futures representation" (sufficient for margin/EOD P&L off exchange
prints) to the "swap representation" (needed for curve risk and APOs)?

The evidence — rates libraries, QuantLib, upstream QuantMath, ETRM vendors;
see the research docs — is unanimous: **never in the data model.** One booked
instrument; the swap representation is *derived structure*, produced
mechanically at a resolve boundary. Fed Funds/SOFR futures are the mature
precedent (structurally identical: futures settling on an average of daily
fixings of a published index):

1. **Resolve** — instrument window + series composition → dated observations,
   each mapped to a concrete underlying contract. Strata makes the boundary
   explicit: `resolve()` turns `OvernightFuture` into
   `ResolvedOvernightFuture`, whose rate field *is* the same rate-computation
   component an OIS swap leg carries. QuantMath puts the same seam at pricer
   construction (`Instrument::fix(fixing_table)`, applied exactly once).
   Resolution happens per pricing request — never at booking, never as a
   second stored trade.
2. **Partition** — per observation, against the valuation date, in the
   *market-data layer*: published fixing (hard error if a past one is
   missing — universal across implementations) vs forward off the futures
   curve. Realized fixings are constants, so they are frozen under bumps and
   delta migrates off the contract as the window elapses — with zero bespoke
   risk code, in every system examined.
3. **Overlay** — any residual gap between the margined future and the
   decomposed expectation is a convexity/basis adjustment (Henrard's
   overnight-futures convexity is the exact analogue): a model concern
   layered on the decomposition, never a representation change. QuantMath
   names the marking half `SpotRequirement::RequiredOnlyForValuation` —
   value to the instrument's own published quote ("valuation has a basis,
   which is considered constant with respect to any risks"), risk through
   the decomposition.

**Descent criterion.** Even the plain BRN future settles on a derived index
(the ICE Brent Index, built from the BFOE cash/forward market) — and nobody
decomposes it, because its constituents live outside our modeled universe;
the series stays an opaque leaf. Descend exactly when the series'
constituents are inside the modeled risk universe *and* the request needs
their sensitivities or joint law. For BFL the constituents are the very
futures curve we already model — which is also *why* risk must descend:
bumping "BFL's own curve" next to BRN positions would break netting on what
is economically one curve family.

The consumer ladder over the single instrument:

| Consumer | Reads | Needs |
|---|---|---|
| Margin / EOD P&L | settle print by future id | nothing new (today's `price_future`) |
| Theoretical PV, missing marks, curve coherence | resolve + partition | fixing store + forward curve |
| Curve risk / scenarios | same, under a shifted `PricingContext` | nothing further — freezing is emergent |
| APO pricing | full decomposition + vol surface | T-W + the seasoned-strike rewrite |

## Proposal (v2)

1. **`SettlementSeries` reference data** — instrument-independent; the series
   knows its own composition (ICE's contract spec puts the roll rule in the
   index definition; Endur models "indices defined in terms of other
   indices"):

   ```rust
   pub struct SettlementSeries {
       pub id: String,                    // "ICE-BRENT-1L"
       pub rule: SeriesRule,
       pub calendar: String,              // publication calendar NAME (no fincal dep)
   }

   pub enum SeriesRule {
       Published { source: String },                 // primary settle/fixing series
       FrontLine { of: IndexRef, roll: RollRule },   // derived front-month selector
       Spread    { legs: Vec<(Decimal, IndexRef)> }, // weighted; ±1 in v1
   }

   pub enum RollRule { RollOnExpiry }                // single-variant v1

   // IndexRef = newtype over String; may name another SettlementSeries
   // (recursion is a feature). An id with no loaded definition is an opaque
   // published leaf — fine for marking; an error only for consumers that
   // need its internals. An IndexRef never names a RateIndex (separate
   // domain, separate namespace).
   ```

2. **`SettlementTerms` on the instrument** — additive, serde-defaulted; the
   window is **explicit dates, bound by the generating driver** (contract
   masterdata — the Strata `OvernightFuture.startDate/endDate` pattern, and
   qloxide's own `FloatingLeg { rate_index_id, start_date, end_date }`
   precedent):

   ```rust
   pub enum SettlementTerms {
       Terminal  { series: IndexRef },                     // EDSP-style fixing
       AverageOf { series: IndexRef, window: (Date, Date) },
       Physical  { delivery: String },
   }

   pub struct Future {
       pub underlying: String,                             // stays: display label
       #[serde(default)]
       pub settlement_terms: Option<SettlementTerms>,
       // NB field name: `settlement` is taken (settlement *timing*)
       ...
   }
   ```

   There is **no `Window` enum and no index "families parameterized by the
   instrument"**: ContractMonth/Balmo are generation-time concerns of the
   driver, which knows the strip; qloxide reads dates. (A balmo window is
   fixed the moment the contract exists — also masterdata.)

3. **The resolve step** — a pure function in the pricing module (instruments
   stay pure data, ARCHITECTURE.md §8.3):
   `resolve(terms, series_defs) → Vec<Observation { date, contract_id }>`,
   applying calendar + roll. A September determination month maps to *two*
   concrete Brent contracts (front until its expiry, next after) — exactly
   the mapping risk needs.

4. **The partition** — `PricingContext::fixing(series_id, date)`: hard error
   on a missing past fixing, forward curve for future dates. Provider-side,
   per the §8.4 doctrine (context = observable market data; scenarios shift
   it). The same fixing store serves swap floating legs (pricing build-order
   step 7) — one mechanism, two consumers.

5. **Population is the driver's job**: qloxide owns the vocabulary;
   qloxide-ice's registry emits the ICE-specific definitions alongside its
   contract mappings (the `BFL` entry already carries this knowledge as a
   comment).

6. **Serialization**: embed-by-value like `Currency` (the existing load-time
   consistency check in `config.rs` extends to series ids). Load validation
   errors on dangling `IndexRef`s and on reference cycles (DFS, same pass).

## What it buys (all hit in practice during the APO work)

- **APOs become structural, not conventional**: an option on a future whose
  terms are `AverageOf { series, window }` is *knowably* an option on an
  average — T₁/T₂ sit on the instrument and the effective-vol-time bracket
  derives from them. The future `AsianOption` pricer needs zero venue config.
- **Settlement verification is generic**: the Sep-26 BFL settle is computable
  from the archived front-line series (machinery already verified
  penny-perfect vs Bloomberg CO1) once the rule is data.
- **Risk freezing/migration is emergent**: realized fixings are constants in
  the context, so bump sensitivity flows only to unfixed dates and delta
  walks off the contract (and across the roll, onto two Brent months) with
  no bespoke risk code.
- **Index algebra**: NOB = `PLATTS-NAPHTHA-CIF-NWE-1L − ICE-BRENT-1L` as a
  weighted-leg `Spread` series — the diff/crack family becomes expressible
  instead of documented.
- **Proxy identity**: two venues' lookalikes sharing a series id (and window)
  are nominally the same economics — `proxy_marks` becomes
  derivable/checkable rather than configured.
- **The observed smile relation gets a home**: the APO smile IS the M+2 flat
  smile (measured within 0.15–1.3 vol pts, Samuelson × window weighting) —
  with spec'd series, "these two surfaces are projections of one process"
  is inferable.

## Migration

Do **not** repurpose the `underlying` string (serialized in every existing
book; legitimate display role; silent format break). Path: add the optional
typed field → drivers populate it → label becomes derivable
(`series.display_name()`) → deprecate the free string in a later 0.x.

## Open questions — v2 status

Resolved by the v2 design / research:

- ~~Window resolution mechanics (load vs pricing time)~~ — neither: bound at
  **generation time** by the driver, as explicit dates on the instrument
  (Strata's Resolved pattern). Balmo is likewise contract masterdata.
- ~~Where does roll live~~ — in the series definition (`FrontLine`), per the
  ICE contract spec ("the pricing quotation rolls to the following month's
  contract").
- ~~Spread scope~~ — weighted legs from day one, populated ±1 until cracks
  need real weights/unit conversions ($/tonne vs $/bbl, 42-gal factor).
- ~~Validation~~ — dangling refs + cycle DFS at load. Independent quick win,
  shippable now: dangling `option.underlying` silently defaults
  `contract_size = 1` today (`portfolio.rs`) — warn first, error later.

Still open:

- **`RateIndex` relation**: recommendation — separate namespaces, an
  `IndexRef` never names a `RateIndex`; confirm and state it in module docs.
- **Convexity overlay** (margined future vs decomposed expectation): when is
  it material for oil averaging futures, and where does it live (pricer
  config vs market data)? Defer until a consumer needs it.
- **`AsianOption`**: new instrument type (recommended — its pricer can
  structurally *require* `AverageOf` terms) vs extending `EuropeanOption`.
- **Calendar source** for in-window day counting: a `CalendarSource` trait on
  the pricing context with a weekday-approximation fallback (leaning), vs
  driver-materialized fixing-date lists.
- **First slice**: M1 = types + serde + embed/consistency + validation (+ the
  `contract_size` warning); M2 = driver population (bitrepo, can trail);
  M3 = `AsianOption` + Turnbull–Wakeman with the seasoned-strike rewrite
  K̂ = (nK − mĀ)/(n−m) (K̂ ≤ 0 ⇒ discounted cash + forward strip).

## References

- [`settlement-index-research-quantmath.md`](settlement-index-research-quantmath.md) —
  upstream QuantMath: `dependencies()`/`DependencyCollector`, `FixingTable`,
  `Instrument::fix` self-decomposition, `RequiredOnlyForValuation`, bump
  machinery with fixing freezing.
- [`settlement-index-research-industry.md`](settlement-index-research-industry.md) —
  Strata `OvernightFuture`/`ResolvedOvernightFuture`, QuantLib
  `OvernightIndexFuture` + Asian machinery, ICE BFL spec & 2012
  swaps-to-futures conversion, Endur/Murex practice, Henrard convexity,
  Hoogland & Neumann seasoned-strike identity.
- bitrepo/icedat `docs/option_model_conventions.md` — the APO vol-convention
  section (T_eff fingerprint, SOFR discounting, flat-surface relation).
- qloxide-ice `src/registry.rs` — the `BFL` entry whose comment this proposal
  turns into data.
- dpdev `ql/brent-apo` — the worked case (real ICE marks via
  `qloxide-ice gen-series`).

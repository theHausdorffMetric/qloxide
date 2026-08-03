# QuantMath (MarcusRainbow/QuantMath) — architectural research

*2026-08-03 · research background for
[`settlement-index-design-note.md`](settlement-index-design-note.md);
compiled by Claude from the upstream sources (README + raw files under
`src/instruments`, `src/risk`, `src/pricers`, `src/models`, `src/data`,
`src/facade`, `src/dates` of <https://github.com/MarcusRainbow/QuantMath>,
master branch).*

Rust library, "Financial maths library for risk-neutral pricing and risk",
MIT, last substantive state ~2019. Strict module hierarchy (README: Facade →
Pricers → Models → Risk → Instruments → Data → Math → Dates → Core),
deliberately with "no backward dependencies". Everything is JSON-serializable
via a tagged-serde type registry (`src/core/factories.rs`,
`src/core/dedup.rs`), so instruments/market data/pricer configs/reports all
round-trip through data — the IT interface is `src/facade/mod.rs`
(`instrument_from_json`, `market_data_from_json`, `calculate(...)`).

## 1. Instrument trait and the dependencies mechanism

`src/instruments/mod.rs` defines a single `pub trait Instrument`
(deliberately no product/index split — see §6). Key method:

```rust
fn dependencies(&self, context: &mut DependencyContext) -> SpotRequirement;
```

`DependencyContext` (same file) is a visitor/callback trait the instrument
writes into: `spot_date()`, `yield_curve(credit_id, high_water_mark)`,
`spot(instrument)`, `forward_curve(instrument, hwm)`,
`vol_surface(instrument, hwm)`, `fixing(id, DateTime)`. Every request carries
a **high-water-mark date** ("beyond which we never ask"), enabling exact
prefetch. The doc comment states the contract: "These calls should match the
calls to the pricing context made during pricing, if this is a Priceable
instrument." The mirror-image trait `PricingContext` (same file) has the same
shape but *returns* data (`RcRateCurve`, `f64` spot, `Arc<Forward>`,
`RcVolSurface`, `correlation`), so declaration and consumption are
structurally parallel.

The return value `SpotRequirement` is a three-state enum — `NotRequired` /
`RequiredOnlyForValuation` / `Required` — expressing whether the instrument's
own quoted spot enters valuation and risk. Notable comment on
`RequiredOnlyForValuation`: "valuation has a basis, which is considered
constant with respect to any risks." Equities return `Required`
(`src/instruments/assets.rs`); options and baskets return `NotRequired` ("A
listed option does not need a spot, because the vols are calibrated to match
the market", `src/instruments/options.rs`).

The concrete collector is `DependencyCollector` in
`src/risk/dependencies.rs`: hash maps of spots, yield-curve/forward/vol
high-water marks (merged via max), `fixings: HashMap<String, Vec<DateTime>>`,
plus a reverse index `forward_id_from_credit_id` (used later to know which
forwards to refetch on a yield bump). Crucially its `spot()` **recurses**:
`let spot_requirement = instrument.dependencies(self);` — so registering one
top-level instrument walks the whole recursive product graph. Example (its
unit test): a European on an equity yields spot on the equity, forward/vol
HWM = expiry, yield-curve HWM = pay date.

## 2. Fixings and the past/future partition

`src/data/fixings.rs`: `FixingTable { fixings_known_until: Date,
fixings_by_id: HashMap<String, Fixings> }`, where `Fixings` maps
`DateTime → f64`; a `DateTime` is date + `TimeOfDay` enum
`{Open, EDSP, Close}` (`src/dates/datetime.rs` — EDSP is explicitly there for
exchange-delivery-settlement-price observations). `get(id, datetime)`
semantics encode the past/future rule: missing fixing **before**
`fixings_known_until` is a hard error; missing fixing today/future is
`Ok(None)`. Doc: "Fixings in the past should always be supplied. Fixings in
the future should never be supplied … Fixings today … may be optionally
supplied."

**Where the partition happens: in the instrument, via self-decomposition,
applied once by the pricer factory.** `Instrument::fix`:

```rust
fn fix(&self, fixing_table: &FixingTable)
    -> Result<Option<Vec<(f64, RcInstrument)>>, qm::Error>
```

An instrument transforms itself into a weighted vector of replacement
instruments given known fixings ("Nothing ever disappears" — README):

- `ForwardStartingEuropean::fix` (`src/instruments/options.rs`): if the
  strike fixing exists, rebuild as a `SpotStartingEuropean` with strike =
  fixing × strike_fraction (then recursively try to fix that too).
- `SpotStartingEuropean::fix`: if the expiry fixing exists, decompose into a
  `ZeroCoupon` cash flow (cash settle) or ZeroCoupon + weighted underlying
  (physical).
- `Basket::fix` (`src/instruments/basket.rs`) delegates to the helper
  `fix_all` (`src/instruments/mod.rs`), which maps weights through member
  decompositions — the composite re-wraps itself as a new `Basket`
  `"{id}:fixed"`.

Both pricer factories apply it exactly once, up front
(`src/pricers/selfpricer.rs`, `src/pricers/montecarlo.rs`):

```rust
// Apply the fixings to the instrument. (This is the last time we need the fixings.)
let instruments = match instrument.fix(&*fixing_table)? {
    Some(fixed) => fixed,
    None => vec!((1.0, instrument)),
};
```

After that, models/pricing contexts never see the fixing table; by
construction everything remaining is future-only.
`MonteCarloDependencies::observation` doc: "All the returned observations
should be in the future (or unfixed, today). Historical observations should
have been handled by the freeze method" (the "freeze method" is `fix` — stale
doc naming). `MonteCarloContext::paths` adds: "all unfixed observations are
effectively in the future (even if they are today) … represented separately
for each path." A `Priceable` guard exists too:
`ForwardStartingEuropean::prices` errors with "You should fix the European
before pricing it, so it does not forward-start in the past". No Asian is
implemented in the repo, but the intended idiom is clear from this machinery:
past averaging observations get absorbed by `fix()` into an
adjusted/decomposed instrument; the model only ever simulates the remaining
future observations.

## 3. Priceable vs MonteCarloPriceable

Both in `src/instruments/mod.rs`, both `: Instrument`, discovered via
optional casts on the base trait (`as_priceable() -> Option<&Priceable>`,
`as_mc_priceable() -> Option<&MonteCarloPriceable>`, default `None`).

- `Priceable` — analytic/self-pricing: `prices(&self, context:
  &PricingContext, dates: &[DateTime], out: &mut [f64])` (batch; `price()` is
  a one-date wrapper). Implemented by `Currency`, `Equity` (in effect via
  forward), `ZeroCoupon`, `Basket`, both Europeans (Black76 via
  `src/math/optionpricing.rs`).
- `MonteCarloPriceable` — three methods: `mc_dependencies(dates, &mut
  MonteCarloDependencies)` (declares per-underlier `observation()`
  date-fractions and `flow()` instruments — the payment vehicles, e.g. a
  ZeroCoupon per payoff); `start_date()` (first date fractional terms fix —
  used by models to know the product is forward-starting);
  `mc_price(&MonteCarloContext)` (reads `paths(underlier)` as an ndarray
  view, computes per-path flow quantities, hands them to `evaluate_flows`).
  The header comment describes MC valuation as "essentially … a map-reduce
  problem."

One instrument can implement both (both Europeans do), and the choice of
engine is the pricer's: `SelfPricerFactory` requires `as_priceable()`,
`MonteCarloPricerFactory` requires `as_mc_priceable()` (`src/pricers/*.rs`);
pricer choice and model choice (`MonteCarloModelFactory`, `src/models/mod.rs`
— implemented: `BlackDiffusion`) are data-driven config. Cross-over
optimization: `Instrument::is_pure_rates()` lets a non-stochastic-rate MC
model value rates-only flow instruments through their `Priceable` interface
instead of per-path (`ZeroCoupon` returns true; used in
`BlackDiffusion::evaluate_flows`, `src/models/blackdiffusion.rs`). Also
`is_driftless()` on `Instrument`: "Instruments that are margined at the
forward level have a value that is effectively driftless. Examples are equity
futures, and options as traded on some exchanges such as Jo'burg and Sao
Paulo."

## 4. Risk: Bumpable, bump-and-revalue, dependency-driven invalidation

`src/risk/mod.rs` defines the stack:

- `trait Bumpable { bump(&mut self, bump: &Bump, save: Option<&mut
  Saveable>) -> Result<bool,_>; dependencies(); context(); new_saveable();
  restore(&Saveable) }` — note `bump` returns **whether anything changed**.
- `trait Pricer: Bumpable + TimeBumpable + PricerClone { price() }`.
- `trait ReportGenerator { generate(&self, pricer: &mut Pricer, saveable,
  unbumped) -> BoxReport }` with concrete
  `DeltaGammaReportGenerator`/`VegaVolgaReportGenerator`/`TimeBumpedReportGenerator`
  and matching serializable `Report`s (`src/risk/deltagamma.rs`,
  `vegavolga.rs`, `timebumped.rs`). Helper `bumped_price(...)`: if `bump()`
  returned false, **skip repricing** and reuse the unbumped price.
- `Bump` itself is a data-layer enum (`src/data/bump.rs`): `Spot(id,
  BumpSpot) | Divs | Borrow | Vol | Yield(credit_id, …) | SpotDate`, with
  per-type bumpers in `src/data/bump*.rs`.

Flow of a bump through the layers: `MarketData`
(`src/risk/marketdata.rs`; the concrete `PricingContext` — hash maps of
spots/yield/borrow/div/vol keyed by string id) implements `Bumpable` by
mutating the addressed map entry, saving the old value into `SavedData`.
`PricingContextPrefetch` (`src/risk/cache.rs`) wraps MarketData plus the
`Arc<DependencyCollector>` and prefetched forwards/vols ("We prefetch the
data, rather than lazily caching it"); on a bump it delegates to MarketData
then **selectively refetches** only affected derived data — e.g. a
spot/div/borrow bump refetches that id's forward; a `Yield(credit_id)` bump
uses `dependencies.forward_id_by_credit_id` to refetch every forward
discounted off that curve. If a pricing call asks for data the instrument
never declared, it fails loudly: "not found (incorrect dependencies?)". One
level up, `BlackDiffusion::bump` (`src/models/blackdiffusion.rs`) does the
same again for simulated paths: only the bumped asset's path array is
regenerated (reusing the stored correlated gaussians), realizing the README
claim "bumping one of those underlyings only results in the affected
Monte-Carlo paths being reevaluated." Every layer's `Saveable` nests the
layer below's, so `restore()` undoes the whole chain cheaply. Report
generators then iterate `dependencies().instruments_clone()` to know *which*
ids to bump — the risk universe itself comes from the instrument's declared
dependency graph, not from the market-data supplied.

**Fixings under bumps:** market-data bumps cannot touch fixings at all —
fixings were consumed at pricer construction and are baked into the
(decomposed) instrument vector, i.e. realized observations are frozen by
construction. The only thing that re-opens them is **time** bumping:
`BumpTime` (`src/risk/bumptime.rs`) rolls the spot date forward;
`update_instruments` looks at `dependencies.fixings(id)` for fixing dates
falling in `[old_spot_date, new_spot_date)`, **synthesizes** fixings for them
from the current market under a chosen `SpotDynamics` (`StickyForward` → read
the forward curve; `StickySpot` → today's spot), builds a temporary
`FixingTable`, re-runs `fix_all`, and if the instrument set changed the
pricer/model is rebuilt from scratch (`TimeBumpable` impls in both pricers).
This is the theta/forward-delta path — the tests in `selfpricer.rs` show
delta jumping to full size once a forward-start's strike date is crossed by a
time bump.

## 5. Derived underlyings (baskets, indices, recursion)

- `Basket` (`src/instruments/basket.rs`) is the composite: weighted
  `Vec<(f64, RcInstrument)>`, itself an `Instrument` and `Priceable` (price =
  weighted sum of members' prices), usable as an *underlying of an option* —
  the options tests price a `SpotStartingEuropean` and
  `ForwardStartingEuropean` on a Basket. Its `dependencies()` recurses into
  members; it returns `SpotRequirement::NotRequired` (its value is defined by
  its components), while registering members whose own requirement demands
  spot. Vanilla options don't care what their underlying is —
  `VanillaOption::prices` obtains the forward by calling
  `underlying.as_priceable()...price(context, expiry)` ("The underlying of an
  option must itself be priceable"), so the pricer descends into derived
  underlyings through the same `Priceable` interface. There is an explicit
  adapter `ForwardFromPriceable` (`src/instruments/mod.rs`) to treat any
  `Priceable` as a `Forward` curve. A TODO comment in
  `VanillaOption::dependencies` flags the known gap: "this forward dependency
  needs to be revisited. The underlying may be a calculated value with no
  spot."
- `Equity` (`src/instruments/assets.rs`) "Represents an equity single name or
  index. Can also be used to represent funds and ETFs" — i.e. a *published*
  index is modeled as an opaque leaf asset with its own spot/vol.
- Vol on derived underlyings is anticipated:
  `Instrument::vol_time_dynamics()` / `vol_forward_dynamics()`
  (ConstantExpiry/RollingExpiry, StickyStrike/StickyDelta) are
  instrument-declared and applied as decorators when
  `MarketData::vol_surface` serves a surface (`src/data/voldecorators.rs`).
- `Dateable`/`Dated` traits (`src/instruments/mod.rs`) sketch undated indices
  (equities, CMS rates) that acquire an expiry when dated — mostly
  scaffolding.
- The README commits to the ambition (§ "Recursive Instruments"): "A basket
  contains composite or quanto underliers, then a dynamic index maintains the
  basket — finally an exotic product is written with the dynamic index as an
  underlying. The library must therefore manage this sort of recursive
  product, whether valuing in Monte-Carlo, analytically or via a finite
  difference engine." Implemented reality: Basket + options-on-basket only;
  no quanto/composite/dynamic-index code exists in the repo.

## 6. Design-philosophy statements (verbatim)

- `src/instruments/mod.rs` (top): "There are a few controversial design
  decisions here. The first is to do with the separation of products from
  indices, which is the case in pricing libraries I have worked with at
  Commerzbank, ABN AMRO, Morgan Stanley and Citi. In practice, I have found
  this distinction irritating and rather specious, so I have classed all
  tradeable instruments together, as Instrument."
- `src/instruments/mod.rs` (above `Priceable`): "The most controversial
  design difference between pricing libraries is whether instruments should
  be allowed to price themselves, or whether pricing is done by some separate
  object… Adherents of the self-pricing instruments include Goldman Sachs…
  Self-pricing makes it easier to construct composite products… However…
  exotic products can often be valued in many different ways. This can easily
  be handled by allowing different combinations of calculator, model and
  product, but a self-pricing product is more limited. We compromise by
  allowing both."
- README: "As products make payments or as dividends go ex, this results in
  the instrument splitting into multiple flows. Nothing ever disappears. The
  SecDB library at Goldman Sachs is famous for taking this philosophy to
  extremes, but QuantMath is at least capable of the same level of
  discipline."
- README: "QuantMath is designed to make it easy to reuse calculations that
  have not changed as a result of a risk bump."
- README: "Models, instruments and risks should be orthogonal, so any can be
  used with any (subject to sensible mathematical restrictions)… QuantMath
  should be runnable purely from serialised state, such as JSON files."

So the "instrument describes itself, pricer interprets" split is explicit but
hybrid: instruments always *describe* (dependencies, MC observations/flows,
fixing-driven decomposition), and *may additionally* self-price when the
value is model-independent; model-dependent interpretation lives in
pricer+model.

## 7. Relevance to "legally a future on a published index, decomposed for risk/vol"

QuantMath has no `Future` instrument, but it has the exact idioms such a
contract would use:

1. **Settlement on a published index fixing** is first-class: fixings are
   keyed by `(instrument id, DateTime)` where `TimeOfDay::EDSP` exists
   specifically for exchange-delivery-settlement-price observations, and the
   futures use case is explicitly anticipated via `Instrument::is_driftless()`
   ("equity futures"). The contract would declare `context.fixing(index_id,
   expiry_EDSP)` in `dependencies()` and implement `fix()` to decompose into
   a `ZeroCoupon` of (fixing − strike) once the EDSP publishes — identical in
   shape to `SpotStartingEuropean::fix`.
2. **The decomposition seam is the instrument, not the pricer.** If the index
   is represented as an opaque `Equity` leaf, risk stops there (delta/vega to
   the index itself: report generators bump exactly the ids in the
   `DependencyCollector`). To get risk/vol through to constituents, you'd
   represent the index as a `Basket`(-like) instrument — then `dependencies()`
   recursion automatically surfaces constituent spots/forwards/vols, bumps to
   any constituent flow through `forward_id_by_credit_id`/refetch machinery,
   and the future's forward is obtained by *pricing* the derived underlying
   via `Priceable` rather than reading a supplied curve. That "underlying is
   a calculated value with no spot" case is acknowledged as not fully
   resolved (the TODO in `VanillaOption::dependencies`), and
   `SpotRequirement::RequiredOnlyForValuation` is the designed halfway house:
   mark the published index price as used "for valuation but not risk" —
   "valuation has a basis, which is considered constant with respect to any
   risks" — i.e. calibrate value to the published quote while risk descends
   into the decomposition.
3. **Realized index fixings stay frozen under bumps** automatically, because
   fixing application happens once at pricer construction; only `BumpTime`
   (theta) re-fixes, and then by *synthesizing* fixings from the bumped
   market under declared `SpotDynamics`.

# Average-price futures in practice — industry research

*2026-08-03 · research background for
[`settlement-index-design-note.md`](settlement-index-design-note.md);
compiled by Claude from the cited web sources. Motivating case: ICE Brent 1st
Line Future (BFL) — legally a future, economically an averaging swap. The
question: when/where do good architectures switch between the "futures
representation" (id + settle prints, enough for margin/EOD P&L) and the
"swap representation" (realized fixings + remaining exposure to concrete
underlying futures, needed for risk and APOs)?*

## Thread 1 — Rates precedent: overnight-index futures (Fed Funds / SOFR)

Fed Funds and SOFR futures are the mature, heavily-engineered precedent for
BFL: legally futures, daily margined, final settle = 100 minus an average
(FF, 1M SOFR) or compounded (3M SOFR) aggregate of daily fixings of a
published index over a reference period.

**OpenGamma Strata** ([OvernightFuture javadoc](https://strata.opengamma.io/apidocs/com/opengamma/strata/product/index/OvernightFuture.html),
[ResolvedOvernightFuture](https://strata.opengamma.io/apidocs/com/opengamma/strata/product/index/ResolvedOvernightFuture.html),
[OvernightAveragedRateComputation](https://strata.opengamma.io/apidocs/com/opengamma/strata/product/rate/OvernightAveragedRateComputation.html)):

- The **product** `OvernightFuture` carries: `securityId`, `currency`,
  `notional`, `accrualFactor` (~1/12), `lastTradeDate`, `startDate`/`endDate`
  (the observation period), `index` (e.g. `USD-FED-FUND`), `accrualMethod`
  (AVERAGED vs COMPOUNDED), `rounding`. Price convention is the exchange's
  `100 − rate` (held as decimal). One instrument; there is no separate "swap
  booking."
- The **Trade → ResolvedTrade** step is the explicit representation switch.
  `resolve(ReferenceData)` produces `ResolvedOvernightFuture`, whose property
  list is `securityId, currency, notional, accrualFactor, lastTradeDate,
  overnightRate, rounding, index` — where `overnightRate` is an
  **`OvernightRateComputation`**, i.e. literally the same rate-computation
  component an OIS swap leg uses. The resolved instrument *contains* the swap
  representation. The computation object holds index + start/end date + rate
  cut-off + the resolved fixing calendar; observation dates are generated
  from these, not stored as a list.
- The **pricer** (`ForwardOvernightAveragedRateComputationFn`,
  [source](https://github.com/OpenGamma/Strata/blob/main/modules/pricer/src/main/java/com/opengamma/strata/pricer/impl/rate/ForwardOvernightAveragedRateComputationFn.java))
  just iterates observation dates and calls `rates.rate(indexObs)` per date
  on the `OvernightIndexRates` obtained from the `RatesProvider`. It contains
  **no** valuation-date logic itself.
- The **realized/unrealized partition lives in the market-data layer**:
  [`DiscountOvernightIndexRates`](https://github.com/OpenGamma/Strata/blob/main/modules/pricer/src/main/java/com/opengamma/strata/pricer/rate/DiscountOvernightIndexRates.java)
  does `if (!observation.getPublicationDate().isAfter(getValuationDate()))
  return historicRate(observation); return rateIgnoringFixings(observation);`
  — historic rates come from a `LocalDateDoubleTimeSeries` of fixings and
  throw `"Unable to get fixing for {} on date {}"` if missing; future rates
  come from discount factors as `(df(start)/df(end) − 1)/accrual`.
- **Curve calibration consumes the decomposition**:
  [`OvernightFutureCurveNode`](https://strata.opengamma.io/apidocs/com/opengamma/strata/market/curve/node/OvernightFutureCurveNode.html)
  builds an `OvernightFutureTrade` from a template + quoted price and
  calibrates the overnight curve through the same rate computation.
  Margining/settlement, by contrast, only needs the quoted/settle price
  against the futures identity.
- Inference from code structure (not a fetched quote): because realized
  fixings return constants and only `rateIgnoringFixings` touches the curve,
  curve delta automatically shrinks onto the unrealized remainder as the
  reference month elapses — no special risk code exists for this.
- Convexity between the margined future and the OIS-style decomposition is
  treated as a model overlay, not a representation issue: Marc Henrard,
  ["Overnight Futures: Convexity Adjustment" (SSRN 3134346)](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=3134346),
  ["Options on overnight futures" (SSRN 4068731)](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=4068731),
  and [his blog](http://multi-curve-framework.blogspot.com/2018/04/overnight-indexed-futures.html)
  — the adjustment has "an Asian flavour due to the composition."

**QuantLib** ([class reference](https://rkapl123.github.io/QLAnnotatedSource/d6/dae/class_quant_lib_1_1_overnight_index_future.html),
[overnightindexfuture.cpp](https://github.com/lballabio/QuantLib/blob/master/ql/instruments/overnightindexfuture.cpp)):

- `OvernightIndexFuture(overnightIndex, valueDate, maturityDate,
  convexityAdjustment = Handle<Quote>(), averagingMethod =
  RateAveraging::Compound)`. Same shape: index reference + explicit
  observation period + averaging method on the instrument.
- `averagedRate()`/`compoundedRate()` iterate the period and branch per
  fixing date: `fixingDate < today` → historical fixing **required** from the
  index's fixing history; `== today` → historical preferred, curve fallback;
  `> today` → forward from the index's forecast curve. The compounded version
  accumulates realized fixings into a product then bridges "from the end of
  the last known fixing to the maturity" telescopically with discount
  factors. `NPV = 100 · (1 − (rate + convexityAdjustment))`.
- Curve building uses
  [`OvernightIndexFutureRateHelper` / `SofrFutureRateHelper`](https://quantlib-python-docs.readthedocs.io/en/latest/thelpers.html)
  taking the averaging method as a parameter ("SOFR fixings are compounded in
  quarterly futures and averaged in monthly ones"); see also
  [test-suite/sofrfutures.cpp](https://github.com/lballabio/QuantLib/blob/master/test-suite/sofrfutures.cpp).
- Notable difference: QuantLib keeps fixing history **on the index object**
  (global `IndexManager`); Strata keeps it in the rates provider passed to
  the pricer. Same partition, different owner — Strata's choice is cleaner
  for scenario/bitemporal risk because market data is an argument rather than
  ambient state.

## Thread 2 — QuantLib's Asian machinery

[`asianoption.hpp`](https://github.com/lballabio/QuantLib/blob/master/ql/instruments/asianoption.hpp)
shows two conventions for communicating realized state, both
instrument-level:

- Legacy: `DiscreteAveragingAsianOption(averageType, runningAccumulator,
  pastFixings, fixingDates, payoff, exercise)` — "takes the running sum or
  product of past fixings, depending on the average type" (pre-aggregated
  realized state + count + the remaining fixing dates).
- Modern: `DiscreteAveragingAsianOption(averageType, fixingDates, payoff,
  exercise, allPastFixings = {})` — the instrument carries the **full**
  fixing schedule, and "during the calculations, the option will compare them
  to the evaluation date to determine which are historic; it will then take
  as many values from allPastFixings as needed and ignore the others."

So: instrument owns explicit fixing dates; the engine performs the
realized/unrealized split against the evaluation date. The evolution from the
first to the second constructor is itself instructive — QuantLib moved from
"pass me the realized aggregate" toward "pass me the whole schedule and let
the pricer partition," converging on the Strata pattern.

## Thread 3 — ICE swaps-to-futures conversion (October 2012) and the BFL spec

- ICE [announced in July 2012](https://www.prnewswire.com/news-releases/intercontinentalexchange-to-transition-cleared-energy-swaps-to-futures-in-october-168443096.html)
  and [completed on October 15, 2012](https://ir.theice.com/press/news-details/2012/ICE-Completes-Transition-of-Energy-Swaps-to-Futures/default.aspx)
  the transition of its entire cleared OTC energy complex to futures: **oil
  products, freight, iron ore and NGL swaps became futures on ICE Futures
  Europe**; North American gas/power/environmental became futures on ICE
  Futures U.S. Existing open interest was transitioned in place; clearing
  stayed at ICE Clear Europe. The driver was Dodd-Frank — October 12, 2012
  was the start of required swap-dealer/MSP compliance, and customers wanted
  "the regulatory certainty of futures"
  ([Energy Business Law summary](https://www.energybusinesslaw.com/2012/08/articles/cftc/ice-announces-conversion-of-cleared-otc-energy-products-to-futures/)).
  So BFL **is** the former Brent 1st-line cleared swap in a futures
  legal/clearing shell — the dual nature is historical fact, not modeling
  artifact.
- [ICE Brent 1st Line Future spec](https://www.ice.com/products/6753532/brent-1st-line-future):
  "A monthly cash settled future based on the ICE daily settlement price for
  Brent Futures." Floating price: "a price in USD and cents per barrel based
  on the average of the settlement prices as made public by ICE for the front
  month ICE Brent Crude Futures contract for each business day in the
  determination period." Roll rule: on the expiration date of the underlying
  delivery month's futures contract, the pricing quotation rolls to the
  following month's contract. 1,000 bbl, cash settled, last trading day =
  last business day of the contract month, tick $0.001, up to 156 consecutive
  months listed. Note the roll rule means a given determination month
  references **two** concrete Brent contracts (front month up to its expiry,
  next month after) — this is exactly the mapping the "swap representation"
  must produce for risk.

## Thread 4 — Commodity risk systems practice

Public evidence here is thinner (vendor internals are not documented openly);
what is evidenced:

- **Endur** ([KWA Analytics, structured gas contract modelling](https://kwa-analytics.com/2014/10/07/endur-structured-gas-contract-modelling-in-endur-3/)):
  averaging lives in **Projection Methods** and "averaging methods" attached
  to pricing **indices**; indices can be "defined in terms of other indices"
  (hierarchical derived indexes — precisely the BFL shape: a monthly-average
  index defined over the front-line Brent index); multi-step averaging
  (daily → monthly → formula) is index-level configuration; "index price
  exposures will be incorporated into Value-at-Risk measures," i.e. risk
  attribution is at the index/curve level, and the article stresses that
  where averaging is placed (index vs contract formula) determines exposure
  and P&L attribution. Deal-side realized fixings are Endur "resets" against
  historical index prices (the reset/historical-price machinery is well known
  among practitioners but no citable public doc for the internals was found —
  treat that detail as unevidenced here).
- **Murex** ([MX.3 E/CTRM brochure](https://www.murex.com/en/insights/brochure/mx3-commodity-trading-and-risk-management)):
  the supported-product list includes "front-month crude futures averaging
  swap, Asians, and swaptions" — note the vendor's own name for the BFL
  payoff is a *swap*. The pricing representation in a cross-asset system is
  the averaging swap; the futures wrapper is a clearing/settlement attribute.
- **SecDB/Athena/Beacon lineage**: public material
  ([Goldman on SecDB](https://www.goldmansachs.com/our-firm/history/moments/1993-secdb),
  [Beacon/Risk.net](https://www.risk.net/risk-magazine/analysis/2479947),
  [eFinancialCareers on SecDB/Quartz/Athena](https://www.efinancialcareers.com/news/2017/03/secdb-quartz-athena-analytics))
  only describes the general architecture: an object database of
  securities/curves/indexes with dependency-graph recomputation. No public
  detail on averaging contracts specifically. Inference (clearly marked as
  such): in a dependency-graph system, "risk flows to underlying curve
  pillars" falls out for free — the settlement-index object depends on the
  underlying futures curve objects, so bumping those pillars reprices the
  average automatically; the derived index is just another node.
- **No** public writeup was found specifically on the pain of dual
  futures/swap representations in ETRM systems. The closest evidenced signal
  is the vocabulary itself: exchanges call these products futures, Murex
  calls the same payoff an averaging swap, and Endur models it as index
  composition — the dual representation is visible across vendors but nobody
  documents the seam.

## Thread 5 — APOs with partially realized averages

- **Turnbull–Wakeman** is the standard moment-matching approach, and
  production implementations expose the in-progress case as instrument
  inputs: MATLAB's [`asianbytw`](https://www.mathworks.com/help/fininst/asianbytw.html)
  takes `AvgDate` ("date averaging period begins," used when `AvgDate <
  Settle`) and `AvgPrice` ("average price of underlying asset at the Settle
  date") — i.e. realized average passed alongside the schedule; likewise
  [`asianbyhhm`](https://www.mathworks.com/help/fininst/asianbyhhm.html) for
  discrete Haug–Haug–Margrabe. The freight/iron-ore literature applies TW
  with zero cost-of-carry precisely because "the average is typically based
  on futures or forward prices"
  ([Springer, Asian options with zero cost-of-carry: EEX options on freight and iron ore futures](https://link.springer.com/article/10.1007/s10203-020-00283-x)).
- The exact mechanism is a payoff rewrite, evidenced in Hoogland & Neumann,
  ["Asians and cash dividends" (arXiv cond-mat/0006133)](https://arxiv.org/abs/cond-mat/0006133),
  §3.7: "It is a well-known fact that the price of a seasoned average price
  option can be expressed in terms of the price of an unseasoned average
  price option with a different strike," with (their continuous notation,
  total window M, remaining window T, A ∝ realized average) the seasoned
  call = (T/M) × unseasoned call at modified strike **K̂ = (MK − A)/T**, and
  "it is possible for the strike K̂ to become negative. In that case, the
  option becomes trivial." Discrete equivalent (elementary algebra, same
  identity): with n fixings total, m realized with mean Ā, payoff =
  ((n−m)/n)·max(Ā_remaining − K̂, 0) with K̂ = (nK − mĀ)/(n−m); K̂ ≤ 0 means
  certain exercise, value = discount × (E[A] − K), i.e. pure cash plus a
  forward strip.
- Consequences (direct corollaries of the rewrite, standard practice): the
  realized portion freezes into a cash-like known quantity with zero delta
  and zero vega; the option becomes a scaled option on the *remaining*
  average, so its effective volatility falls and its delta redistributes
  across the underlying contracts of the remaining fixing dates — for a BFL
  APO, onto the concrete front-month (and, past the roll, next-month) Brent
  futures for the remaining business days. On hedge-portfolio dynamics see
  also Jacques, ["On the hedging portfolio of Asian options" (ASTIN Bulletin)](https://www.casact.org/sites/default/files/database/astin_vol26no2_165.pdf).

## Distillation — the common architectural pattern

The evidence across rates and commodities is consistent and fairly sharp:

**1. One booked instrument, never two.** Nobody books a futures
representation and a swap representation. The instrument is the future:
identity (security id / contract symbol), a **reference to the derived
index**, the **observation/determination period**, and the **averaging
method**. Strata's `OvernightFuture` is the cleanest exemplar; QuantLib's
`OvernightIndexFuture` matches; ICE's own contract spec is exactly this tuple
(front-month Brent settlement index + calendar-month determination period +
arithmetic average + roll rule).

**2. The "swap representation" is derived structure, materialized at the
resolve/pricing boundary.** Strata makes the switch point explicit:
`resolve(ReferenceData)` turns the terse product into
`ResolvedOvernightFuture` whose `overnightRate` field **is** an
`OvernightRateComputation` — the identical component a swap leg carries. That
is the answer to "when do good architectures switch": at resolution/pricing
time, mechanically, from the instrument's period + the index's calendar and
composition rules — not at booking, and not by maintaining a parallel trade.

**3. The index is reference data that knows its own composition.** The roll
rule ("front month, rolling to next on expiry") belongs to the index object,
not the trade — Endur's "indices defined in terms of other indices" and
Strata's index + resolved-calendar design agree. This is what lets risk land
on *concrete* underlying futures: the index maps each observation date to a
specific underlying contract.

**4. The realized/unrealized partition is done at pricing time against the
valuation date, per observation, by publication date.** Realized → constant
from a fixing store (hard error if missing — every implementation treats a
missing past fixing as a data error, not something to forecast); unrealized →
forecast from the underlying curve. Where the fixing store lives varies
(Strata: time series inside the rates provider; QuantLib rates: on the index
singleton; QuantLib Asians and MATLAB TW: on the instrument as
accumulator/realized-average; Endur: historical index prices/resets), and
Strata's provider-side placement is the most defensible because the pricer
and instrument stay stateless. Because realized fixings return constants and
only unrealized observations touch curves, **curve delta migrates off the
contract automatically** as the determination period elapses — no bespoke
risk logic exists in any of these systems for that.

**5. The two consumers read different projections of the same object.**
Margin/settlement P&L needs only the futures identity and the exchange's
published settle (Strata prices in the exchange's 100 − rate convention
precisely so marks reconcile against prints). Curve calibration, theoretical
PV, sensitivities, and option pricing consume the decomposition (Strata
`OvernightFutureCurveNode`, QuantLib `SofrFutureRateHelper`). The gap between
the two — futures margining vs the decomposed expectation — is handled as a
**convexity adjustment overlay** (a model concern layered on the
decomposition, per Henrard), not as a representation change.

**6. Options confirm the pattern from the other side.** APO machinery
(QuantLib Asians, TW implementations) puts the fixing schedule on the
instrument and the realized average in as state; the seasoned option is
exactly a rescaled unseasoned option on the remaining average with adjusted
strike K̂ = (nK − mĀ)/(n−m), degenerating to discounted cash when K̂ ≤ 0. The
vol surface belongs to the derived index's remaining-average distribution;
delta and vega live only on the unrealized fixings' underlying contracts.

For qloxide's BFL case this maps to: store one instrument (BFL id +
determination month + reference to a "Brent front-line settlement" index
object owning the roll rule); EOD P&L reads exchange settles keyed by the
futures id and never expands anything; a `resolve`-style step expands the
determination month into dated observations when (and only when) risk,
calibration, or APO pricing is requested; each observation resolves via the
index to a concrete Brent contract and is partitioned by valuation date —
published settles from the fixing store, the remainder from the futures
curve.

Evidence status: everything quoted above (class structures, code logic,
contract spec, press releases, the seasoned-strike formula) is fetched and
cited. Marked as inference: the automatic delta-migration claim for Strata
(follows from code structure, sensitivity method not fetched), SecDB-lineage
handling of averaging contracts specifically, and the characterization of
Endur reset internals beyond the KWA article.

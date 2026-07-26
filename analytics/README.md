# qloxide-analytics

Option-implied **risk-neutral density** extraction over qloxide market
data: no-arbitrage pre-filter diagnostics → Fengler (2009) constrained
QP on forward call prices (globally non-negative density by
construction) → Breeden–Litzenberger extraction with exact piecewise
moments → GPD tail grafting under the Bollinger–Melick–Thomas repricing
criterion (`E[S] = F` exact) → Bliss–Panigirtzoglou perturbation bands.

The mathematics, the numerical gotchas, and the reference list:
[`METHODS.md`](METHODS.md).

This crate sits outside the qloxide core on purpose — the core stays a
lean pricing library (libm-only); the conic-QP solver (clarabel) lives
here. Input seam: `slice::SmileSlice` — today built from calibrated
`VolSurface::Grid` nodes, later from raw-quote sidecars when tier-1
quote data ships. Figure rendering and the CLI live downstream (the
`rndoxide` binary in the ideas repo).

```rust
use qloxide_analytics::{density, fengler, slice, tails};

let history = slice::load_history("market.json".as_ref())?;   // + sibling vols.json
let day = slice::day_record(&history, None)?;                 // default: last day
let smile = slice::SmileSlice::from_market(day, &slice::grid_ids(day)[0])?;
let fit = fengler::fit(&smile.strikes, &smile.calls, smile.f,
                       &fengler::FenglerConfig::default())?;
let dens = density::extract(&fit);
let stitched = tails::graft(&smile, &fit, &dens, 0.05, 0.02)?; // α_L/α_R attachments
```

Tests: `cargo test -p qloxide-analytics` — includes the lognormal
ground-truth recovery (flat-vol slice → analytic density to <0.1%
interior, <1% in the ±2σ wings).

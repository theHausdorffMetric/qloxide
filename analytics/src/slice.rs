//! Per-expiry smile slice: the discrete quotes an RND extraction consumes.

use qloxide::dates::Date;
use qloxide::instruments::PutOrCall;
use qloxide::market_data::{MarketData, MarketHistory, VolSurface};
use qloxide::pricing::black76::{Black76Params, black76_price};

use crate::{Error, Result};

/// A single-expiry smile as discrete forward-space call quotes.
///
/// Strikes are the vol-grid *nodes* — the listed strikes the surface was
/// calibrated to. The grid interpolates linearly in vol between nodes, so
/// sampling anywhere else would inject artificial kinks exactly where the
/// density takes its second derivative; only the nodes carry information.
///
/// `calls` are undiscounted (forward) Black76 prices at the node vols:
/// with C̃ = e^{rT}·C the Breeden-Litzenberger identity is simply
/// q(K) = ∂²C̃/∂K², no discounting anywhere downstream.
#[derive(Clone, Debug)]
pub struct SmileSlice {
    pub underlying: String,
    pub valuation_date: String,
    /// Futures/forward price.
    pub f: f64,
    /// Time to expiry in years.
    pub t: f64,
    /// Strikes at the grid nodes, strictly increasing.
    pub strikes: Vec<f64>,
    /// Node lognormal vols.
    pub vols: Vec<f64>,
    /// Forward (undiscounted) call prices at the nodes.
    pub calls: Vec<f64>,
}

impl SmileSlice {
    /// Extract the slice for `id` from loaded market data.
    ///
    /// Requires a single-tenor `Grid` surface — the data qloxide-ice
    /// produces today. Multi-tenor extraction belongs to the calendar-
    /// constraint phase, not this PoC.
    pub fn from_market(market: &MarketData, id: &str) -> Result<SmileSlice> {
        let surface = market.vol_surface(id)?;
        let VolSurface::Grid {
            tenors,
            moneyness,
            vols,
        } = surface
        else {
            return Err(Error::Market(format!(
                "'{id}': need a Grid surface, found a non-Grid variant"
            )));
        };
        if tenors.len() != 1 {
            return Err(Error::Market(format!(
                "'{id}': expected a single tenor, found {} (multi-tenor needs calendar constraints)",
                tenors.len()
            )));
        }
        let t = tenors[0];
        let f = market.market_price(id)?;

        let mut strikes = Vec::with_capacity(moneyness.len());
        let mut node_vols = Vec::with_capacity(moneyness.len());
        let mut calls = Vec::with_capacity(moneyness.len());
        for (m, sigma) in moneyness.iter().zip(&vols[0]) {
            let k = f * m.exp();
            // r = 0 makes black76_price return the undiscounted (forward)
            // premium: e^{-rT}[F·N(d1) - K·N(d2)] with the discount = 1.
            let c = black76_price(
                Black76Params {
                    f,
                    k,
                    t,
                    r: 0.0,
                    sigma: *sigma,
                },
                PutOrCall::Call,
            )?;
            strikes.push(k);
            node_vols.push(*sigma);
            calls.push(c);
        }

        Ok(SmileSlice {
            underlying: id.to_string(),
            valuation_date: market.valuation_date().to_string(),
            f,
            t,
            strikes,
            vols: node_vols,
            calls,
        })
    }

    /// Vol at the node nearest the money (log-moneyness 0).
    pub fn atm_vol(&self) -> f64 {
        let mut best = (f64::MAX, 0.0);
        for (k, v) in self.strikes.iter().zip(&self.vols) {
            let dist = (k / self.f).ln().abs();
            if dist < best.0 {
                best = (dist, *v);
            }
        }
        best.1
    }
}

/// IDs of the Grid vol surfaces in a day record, sorted.
pub fn grid_ids(md: &MarketData) -> Vec<String> {
    let mut ids: Vec<String> = md
        .vol_surface_ids()
        .into_iter()
        .filter(|id| matches!(md.vol_surface(id), Ok(VolSurface::Grid { .. })))
        .map(str::to_string)
        .collect();
    ids.sort();
    ids
}

/// Load a one-file market history (qloxide arch §9: `market.json` *is*
/// the history), merging the sibling risk-tier vols history
/// (`market.json` ↔ `vols.json`) day-wise when it exists — book day
/// records are vol-free; the Grids live in the vols file.
pub fn load_history(path: &std::path::Path) -> Result<MarketHistory> {
    let mut history: MarketHistory = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    if let Some(vols_path) = sibling_vols_path(path)
        && vols_path.exists()
    {
        let vols: MarketHistory = serde_json::from_str(&std::fs::read_to_string(&vols_path)?)?;
        history.merge_days(vols)?;
    }
    history.validate()?;
    Ok(history)
}

/// Select one day record (default: the last).
pub fn day_record(history: &MarketHistory, day: Option<Date>) -> Result<&MarketData> {
    Ok(match day {
        Some(d) => history.day(d)?,
        None => history.last()?,
    })
}

/// The risk-tier vols file conventionally paired with a market file.
fn sibling_vols_path(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let name = path.file_name()?.to_str()?;
    (name == "market.json").then(|| path.with_file_name("vols.json"))
}

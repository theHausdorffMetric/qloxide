//! Option-implied risk-neutral density extraction over qloxide market data.
//!
//! Pipeline: discrete quotes from a calibrated `qloxide::VolSurface::Grid`
//! → no-arbitrage pre-filter diagnostics → Fengler (2009) constrained QP
//! on forward call prices → Breeden-Litzenberger density with
//! certificates → GPD tail grafting (Bollinger-Melick-Thomas repricing
//! criterion) → Bliss-Panigirtzoglou perturbation bands. Everything works
//! in forward (undiscounted) space, so no discount factor appears
//! anywhere in the extraction.
//!
//! The mathematics, its failure modes, and the reference list live in
//! `METHODS.md` next to this crate — a self-contained distillation of the
//! research bundle that settled the methodology. Input seam:
//! [`slice::SmileSlice`] is the canonical
//! discrete slice every stage consumes — today built from Grid nodes
//! ([`slice::SmileSlice::from_market`]); a raw-quote sidecar constructor
//! plugs in there when tier-1 quote data ships.
//!
//! This crate deliberately sits *outside* the qloxide core: the core
//! stays a lean pricing library (libm-only, cargo-deny-gated), while the
//! conic-QP solver dependency (clarabel) lives here. Figure rendering
//! stays a CLI-layer concern (the `rndoxide` binary in the ideas repo).

pub mod density;
pub mod fengler;
pub mod perturb;
pub mod prefilter;
pub mod slice;
pub mod tails;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("market data: {0}")]
    Market(String),
    #[error("solver: {0}")]
    Solver(String),
    #[error(transparent)]
    Qloxide(#[from] qloxide::core::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

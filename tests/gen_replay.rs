//! Engine validation without a venue (architecture §10, task 74):
//! replaying each committed example book through the generic engine must
//! reproduce its `market.json` (and `vols.json`) byte-identically, and
//! the replay source must pass the I5 conformance kit.
//!
//! The same property is the driver-equivalence gate on the icedat
//! machine once qloxide-ice sits on the `MarketSource` trait.

use std::path::Path;
use std::sync::Arc;

use qloxide::generator::{self, GenParams, conformance, replay::ReplaySource};
use qloxide::instruments::FinancialInstrument;
use qloxide::market_data::MarketHistory;
use qloxide::trades::Deal;

fn examples() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples")
}

fn read_history(path: &Path) -> MarketHistory {
    let json = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    serde_json::from_str(&json).unwrap_or_else(|e| panic!("{path:?}: {e}"))
}

fn read_instruments(dir: &Path, files: &[&str]) -> Vec<Arc<dyn FinancialInstrument>> {
    files
        .iter()
        .flat_map(|f| {
            let json = std::fs::read_to_string(dir.join(f)).unwrap();
            serde_json::from_str::<Vec<Arc<dyn FinancialInstrument>>>(&json).unwrap()
        })
        .collect()
}

fn read_deals(dir: &Path) -> Vec<Deal> {
    let json = std::fs::read_to_string(dir.join("deals.json")).unwrap();
    serde_json::from_str(&json).unwrap()
}

/// Replay one example book: byte-identical regeneration + clean I5 run.
/// The ICE-data books are excluded from the published package (licensed
/// data); a missing directory skips rather than fails.
fn replay_book(book: &str, instrument_files: &[&str]) {
    let dir = examples().join(book);
    if !dir.exists() {
        eprintln!("{book}: example not present (excluded from the published package) — skipping");
        return;
    }
    let committed_market = std::fs::read_to_string(dir.join("market.json")).unwrap();
    let market: MarketHistory = serde_json::from_str(&committed_market).unwrap();
    let vols_path = dir.join("vols.json");
    let committed_vols = vols_path
        .exists()
        .then(|| std::fs::read_to_string(&vols_path).unwrap());
    let vols = committed_vols
        .as_deref()
        .map(|json| serde_json::from_str::<MarketHistory>(json).unwrap());

    let instruments = read_instruments(&dir, instrument_files);
    let deals = read_deals(&dir);
    let params = GenParams {
        from: market.first_day(),
        through: market.last_day().unwrap(),
    };

    let source = ReplaySource::new(read_history(&dir.join("market.json")), vols).unwrap();
    let generated = generator::generate(&source, &instruments, Some(&deals), &params).unwrap();

    assert_eq!(
        generated.market.to_canonical_json().unwrap(),
        committed_market,
        "{book}: regenerated market.json differs from the committed file"
    );
    match (&generated.vols, committed_vols) {
        (None, None) => {}
        (Some(generated), Some(committed)) => assert_eq!(
            generated.to_canonical_json().unwrap(),
            committed,
            "{book}: regenerated vols.json differs from the committed file"
        ),
        (generated, committed) => panic!(
            "{book}: vols mismatch — generated {:?}, committed {:?}",
            generated.is_some(),
            committed.is_some()
        ),
    }

    let findings = conformance::check(&source, &instruments, Some(&deals), &params).unwrap();
    assert!(findings.is_empty(), "{book}: I5 findings: {findings:#?}");
}

/// The synthetic public book (`examples/example-public/`): the one
/// replay that always runs, in the repo and in the published package.
#[test]
fn replay_example_public() {
    replay_book("example-public", &["instruments.json"]);
}

#[test]
fn replay_brent_condor() {
    replay_book("brent-condor", &["instruments.json"]);
}

#[test]
fn replay_brent_option() {
    replay_book("brent-option", &["instruments.json"]);
}

#[test]
fn replay_brent_timespread() {
    replay_book("brent-timespread", &["instruments.json"]);
}

/// The legacy brent book is hand-written, not generator output (settle-time
/// as_of, no header stamps) — byte-identical regeneration is a property of
/// generated files only, so it runs the conformance kit alone.
#[test]
fn conformance_brent_legacy() {
    let dir = examples().join("brent");
    if !dir.exists() {
        eprintln!("brent: example not present (excluded from the published package) — skipping");
        return;
    }
    let market = read_history(&dir.join("market.json"));
    let vols = read_history(&dir.join("vols.json"));
    let instruments = read_instruments(&dir, &["instruments.json", "options.json"]);
    let deals = read_deals(&dir);
    let params = GenParams {
        from: market.first_day(),
        through: market.last_day().unwrap(),
    };
    let source = ReplaySource::new(market, Some(vols)).unwrap();
    let findings = conformance::check(&source, &instruments, Some(&deals), &params).unwrap();
    assert!(findings.is_empty(), "brent: I5 findings: {findings:#?}");
}

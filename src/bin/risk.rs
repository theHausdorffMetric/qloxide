//! `qloxide-risk` — the model world (architecture §9): greeks,
//! calibration residuals, model marks and book value, scenario overlays
//! via `--market` (gen-bumps files; settles stripped, so scenarios
//! structurally cannot official-mark).

use std::path::PathBuf;

use clap::Parser;

use qloxide::cli;

#[derive(Parser)]
#[command(
    name = "qloxide-risk",
    version,
    about = "Model-world risk reports: greeks, scenarios, model marks",
    after_long_help = cli::config_help()
)]
struct Cli {
    /// Path to TOML pricing config
    #[arg(long)]
    config: PathBuf,

    /// Replace the config's market_data with this single market file
    /// (scenario runs: price the book against a bumped market).
    #[arg(long)]
    market: Option<PathBuf>,

    /// Reports to run (overrides config).
    #[arg(long = "report", long_help = cli::report_long_help(&cli::RISK))]
    reports: Vec<String>,
}

fn main() {
    let args = Cli::parse();
    cli::run(
        &cli::RISK,
        &args.config,
        args.market.as_deref(),
        &args.reports,
    )
}

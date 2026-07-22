//! `qloxide-risk` — the model world (architecture §9): greeks,
//! calibration residuals, model marks and book value, scenario overlays
//! via `--market` (one-day bump histories; settles stripped, so
//! scenarios structurally cannot official-mark).

use std::path::PathBuf;

use clap::Parser;

use qloxide::cli;
use qloxide::dates::Date;

#[derive(Parser)]
#[command(
    name = "qloxide-risk",
    version,
    about = "Model-world risk reports: greeks, scenarios, model marks",
    after_long_help = cli::RISK.config_help
)]
struct Cli {
    /// Path to the risk TOML config
    #[arg(long)]
    config: PathBuf,

    /// Replace the config's market with this single market file
    /// (scenario runs: a self-contained one-day bump history).
    #[arg(long)]
    market: Option<PathBuf>,

    /// Value against this day of the market history instead of the last.
    #[arg(long, value_name = "DATE")]
    as_of: Option<Date>,

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
        args.as_of,
        &args.reports,
    )
}

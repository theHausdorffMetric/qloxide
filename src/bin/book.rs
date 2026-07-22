//! `qloxide-book` — the official world (architecture §9): listings,
//! positions compression, settle-primary P&L. No market override:
//! this binary structurally cannot emit model numbers.

use std::path::PathBuf;

use clap::Parser;

use qloxide::cli;

#[derive(Parser)]
#[command(
    name = "qloxide-book",
    version,
    about = "Official-marks book reports: listings, positions, P&L",
    after_long_help = cli::config_help()
)]
struct Cli {
    /// Path to TOML pricing config
    #[arg(long)]
    config: PathBuf,

    /// Reports to run (overrides config).
    #[arg(long = "report", long_help = cli::report_long_help(&cli::BOOK))]
    reports: Vec<String>,
}

fn main() {
    let args = Cli::parse();
    cli::run(&cli::BOOK, &args.config, None, &args.reports)
}

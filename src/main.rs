use std::path::PathBuf;
use std::process;

use clap::Parser;

use qloxide::config;
use qloxide::reports;

#[derive(Parser)]
#[command(name = "qloxide", about = "Financial instrument pricing")]
struct Cli {
    /// Path to TOML pricing config
    #[arg(long)]
    config: PathBuf,

    /// Reports to run (overrides config). Options: instruments, deals, pnl
    #[arg(long = "report")]
    reports: Vec<String>,
}

fn main() {
    let cli = Cli::parse();

    let portfolio = config::load(&cli.config).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1);
    });

    for w in &portfolio.warnings {
        eprintln!("warning: {w}");
    }

    let report_names = if cli.reports.is_empty() {
        &portfolio.reports
    } else {
        &cli.reports
    };

    for name in report_names {
        let output = reports::run(name, &portfolio).unwrap_or_else(|e| {
            eprintln!("error: {e}");
            process::exit(1);
        });
        print!("{output}");
    }
}

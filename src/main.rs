use std::path::PathBuf;
use std::process;

use clap::Parser;
use clap::builder::PossibleValuesParser;

use qloxide::config;
use qloxide::reports;

#[derive(Parser)]
#[command(
    name = "qloxide",
    about = "Financial instrument pricing",
    after_long_help = config_help()
)]
struct Cli {
    /// Path to TOML pricing config
    #[arg(long)]
    config: PathBuf,

    /// Reports to run (overrides config).
    #[arg(long = "report", value_parser = report_value_parser(), long_help = report_long_help())]
    reports: Vec<String>,
}

/// Validates `--report` values and self-documents the choices in `--help`,
/// driven by the single source of truth in [`reports::descriptions`].
fn report_value_parser() -> PossibleValuesParser {
    PossibleValuesParser::new(reports::available())
}

/// Long help for `--report`: each report name and what it produces.
fn report_long_help() -> String {
    let width = reports::descriptions()
        .iter()
        .map(|(name, _)| name.len())
        .max()
        .unwrap_or(0);
    let mut help = String::from("Reports to run (overrides config).\n");
    for (name, desc) in reports::descriptions() {
        help.push_str(&format!("\n  {name:<width$}  {desc}"));
    }
    help
}

/// Trailing `--help` section documenting the TOML config schema.
fn config_help() -> String {
    "CONFIG (TOML):\n  \
     instruments   = [\"path.json\", ...]   # instrument definitions\n  \
     deals         = [\"path.json\", ...]   # trades\n  \
     market_data   = [\"path.json\", ...]   # quotes, curves, valuation date (optional: static reports run without it)\n  \
     market_series = \"series/manifest.json\" # optional historical series; enables settle-completeness checks\n  \
     reports       = [\"pnl\", ...]         # default reports when --report is omitted\n\n  \
     [[waivers]]                          # optional: downgrade known series holes to warnings\n  \
     from = \"2020-01-01\"\n  \
     through = \"2020-09-30\"\n  \
     reason = \"upstream data gap\"\n\n  \
     Paths are resolved relative to the config file's directory."
        .to_string()
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
    // Integrity errors don't abort: static reports still render for
    // diagnosis; valuation reports refuse on their own.
    for e in &portfolio.integrity_errors {
        eprintln!("integrity error: {e}");
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

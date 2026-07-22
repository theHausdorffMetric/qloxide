//! Shared driver for the qloxide binaries.
//!
//! The executable surface splits along trust tiers (architecture §9):
//! `qloxide-book` serves the official world (listings, positions,
//! settle-primary P&L; no market override), `qloxide-risk` the model world
//! (greeks, scenario overlays via `--market`). Both are thin clap wrappers
//! around [`run`]; the tier decides which reports a binary may serve and
//! where a mistyped request gets pointed.

use std::path::Path;
use std::process;

use crate::config;
use crate::reports;

/// One binary's slice of the report registry.
pub struct Tier {
    /// Binary name, e.g. `qloxide-book`.
    pub bin: &'static str,
    /// The sibling binary a wrong-tier report request is pointed to.
    pub other_bin: &'static str,
    /// `(name, description)` pairs this binary serves.
    pub reports: &'static [(&'static str, &'static str)],
    /// Reports run when neither `--report` nor the config names any from
    /// this tier. Empty = run nothing (the book tier: an explicit config
    /// is the contract; the risk tier defaults to its one report).
    pub fallback: &'static [&'static str],
    /// Tier-specific loader: the book tier ignores `vol_data` and warns on
    /// surfaces no position needs; the risk tier merges the `vol_data`
    /// enrichment (architecture §9).
    pub load: fn(&Path, Option<&Path>) -> crate::core::Result<config::Portfolio>,
}

/// The official world: `qloxide-book`.
pub const BOOK: Tier = Tier {
    bin: "qloxide-book",
    other_bin: "qloxide-risk",
    reports: reports::BOOK_REPORTS,
    fallback: &[],
    load: config::load_with_market,
};

/// The model world: `qloxide-risk`.
pub const RISK: Tier = Tier {
    bin: "qloxide-risk",
    other_bin: "qloxide-book",
    reports: reports::RISK_REPORTS,
    fallback: &["risk"],
    load: config::load_risk,
};

impl Tier {
    fn serves(&self, name: &str) -> bool {
        self.reports.iter().any(|(n, _)| *n == name)
    }
}

/// Long help for `--report`: each report this tier serves.
pub fn report_long_help(tier: &Tier) -> String {
    let width = tier
        .reports
        .iter()
        .map(|(name, _)| name.len())
        .max()
        .unwrap_or(0);
    let mut help = String::from("Reports to run (overrides config).\n");
    for (name, desc) in tier.reports {
        help.push_str(&format!("\n  {name:<width$}  {desc}"));
    }
    help
}

/// Trailing `--help` section documenting the TOML config schema.
pub fn config_help() -> String {
    "CONFIG (TOML):\n  \
     instruments   = [\"path.json\", ...]   # instrument definitions\n  \
     deals         = [\"path.json\", ...]   # trades\n  \
     market_data   = [\"path.json\", ...]   # quotes, curves, valuation date (optional: static reports run without it)\n  \
     market_series = \"series/manifest.json\" # optional historical series; enables settle-completeness checks\n  \
     vol_data      = [\"vols.json\", ...]   # risk-tier vol surfaces; merged by qloxide-risk only, ignored by qloxide-book\n  \
     reports       = [\"pnl\", ...]         # default reports when --report is omitted (filtered to this binary's tier)\n\n  \
     [[waivers]]                          # optional: downgrade known series holes to warnings\n  \
     from = \"2020-01-01\"\n  \
     through = \"2020-09-30\"\n  \
     reason = \"upstream data gap\"\n\n  \
     Paths are resolved relative to the config file's directory."
        .to_string()
}

/// Load the portfolio and run the requested reports for one tier.
///
/// Explicit `--report` values must belong to the tier — a report from the
/// sibling tier errors with a pointer to the other binary. Config-supplied
/// defaults are filtered to the tier (sibling-tier names silently, unknown
/// names with a warning), falling back to [`Tier::fallback`] when nothing
/// remains. Exits the process on error, matching the previous
/// single-binary behavior.
pub fn run(tier: &Tier, config_path: &Path, market: Option<&Path>, requested: &[String]) -> ! {
    for name in requested {
        if !tier.serves(name) {
            if let Some(desc) = reports::describe(name) {
                eprintln!(
                    "error: report '{name}' ({desc}) is not served by {} — run {} instead",
                    tier.bin, tier.other_bin
                );
            } else {
                eprintln!(
                    "error: unknown report '{name}'. available in {}: {}",
                    tier.bin,
                    tier.reports
                        .iter()
                        .map(|(n, _)| *n)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            process::exit(2);
        }
    }

    let portfolio = (tier.load)(config_path, market).unwrap_or_else(|e| {
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

    // Configs are shared between the binaries, so sibling-tier names in the
    // config's report list are expected and filtered silently; only names
    // no binary serves get a warning.
    let report_names: Vec<&str> = if requested.is_empty() {
        let (kept, skipped): (Vec<&str>, Vec<&str>) = portfolio
            .reports
            .iter()
            .map(String::as_str)
            .partition(|n| tier.serves(n));
        for name in skipped {
            if reports::describe(name).is_none() {
                eprintln!("warning: unknown report '{name}' in config — skipped");
            }
        }
        if kept.is_empty() {
            tier.fallback.to_vec()
        } else {
            kept
        }
    } else {
        requested.iter().map(String::as_str).collect()
    };

    for name in report_names {
        let output = reports::run(name, &portfolio).unwrap_or_else(|e| {
            eprintln!("error: {e}");
            process::exit(1);
        });
        print!("{output}");
    }
    process::exit(0);
}

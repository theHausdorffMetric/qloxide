//! Shared driver for the qloxide binaries.
//!
//! The executable surface splits along trust tiers (architecture §9):
//! `qloxide-book` serves the official world (listings, positions,
//! settle-primary P&L; no market override), `qloxide-risk` the model world
//! (greeks, scenario overlays via `--market`). Each tier has its own
//! config schema — `book.toml` (strict) vs `risk.toml` — so a config's
//! report list is tier-owned; a report from the sibling tier is an error
//! pointing at the other binary, never silently filtered.

use std::path::Path;
use std::process;

use crate::config;
use crate::dates::Date;
use crate::reports;

/// One binary's slice of the report registry.
pub struct Tier {
    /// Binary name, e.g. `qloxide-book`.
    pub bin: &'static str,
    /// The sibling binary a wrong-tier report request is pointed to.
    pub other_bin: &'static str,
    /// `(name, description)` pairs this binary serves.
    pub reports: &'static [(&'static str, &'static str)],
    /// Reports run when neither `--report` nor the config names any.
    /// Empty = run nothing (the book tier: an explicit config is the
    /// contract; the risk tier defaults to its one report).
    pub fallback: &'static [&'static str],
    /// Tier-specific loader: the book tier takes no market override and
    /// warns on surfaces no position needs; the risk tier merges the
    /// `vol_data` enrichment (architecture §9).
    pub load: fn(&Path, Option<&Path>, Option<Date>) -> crate::core::Result<config::Portfolio>,
    /// The tier's config schema, appended to `--help`.
    pub config_help: &'static str,
}

fn load_book_tier(
    config_path: &Path,
    _market: Option<&Path>,
    as_of: Option<Date>,
) -> crate::core::Result<config::Portfolio> {
    config::load_book(config_path, as_of)
}

/// The official world: `qloxide-book`.
pub const BOOK: Tier = Tier {
    bin: "qloxide-book",
    other_bin: "qloxide-risk",
    reports: reports::BOOK_REPORTS,
    fallback: &[],
    load: load_book_tier,
    config_help: BOOK_CONFIG_HELP,
};

/// The model world: `qloxide-risk`.
pub const RISK: Tier = Tier {
    bin: "qloxide-risk",
    other_bin: "qloxide-book",
    reports: reports::RISK_REPORTS,
    fallback: &["risk"],
    load: config::load_risk,
    config_help: RISK_CONFIG_HELP,
};

const BOOK_CONFIG_HELP: &str = "CONFIG (book.toml — strict schema: unknown or risk-tier keys are errors):\n  \
     instruments = [\"path.json\", ...]  # instrument definitions\n  \
     deals       = [\"path.json\", ...]  # trades\n  \
     market      = \"market.json\"       # one-file market history {source, generator, skipped_days, days: [...]}\n  \
                                       # optional: static reports run without it\n  \
     reports     = [\"pnl\", ...]        # default reports when --report is omitted\n\n  \
     [[waivers]]                       # optional: downgrade known history holes to warnings\n  \
     from = \"2020-01-01\"\n  \
     through = \"2020-09-30\"\n  \
     reason = \"upstream data gap\"\n\n  \
     [proxy_marks]                     # optional: uncleared lookalike -> cleared twin's settle\n  \
     \"BIL-BRN-U26\" = \"ICE-B-U26\"\n\n  \
     Paths are resolved relative to the config file's directory.";

const RISK_CONFIG_HELP: &str = "CONFIG (risk.toml — lists the same data files as the book, plus risk enrichment):\n  \
     instruments = [\"path.json\", ...]  # same instrument definitions as the book\n  \
     deals       = [\"path.json\", ...]  # same trades as the book\n  \
     market      = \"market.json\"       # the book's market history\n  \
     vol_data    = [\"vols.json\", ...]  # risk-tier vol histories, merged day-wise\n  \
     reports     = [\"risk\", ...]       # default reports when --report is omitted\n\n  \
     [[waivers]] / [proxy_marks]       # duplicated from book.toml where the risk\n  \
                                       # report needs the policy\n\n  \
     Paths are resolved relative to the config file's directory.";

impl Tier {
    fn serves(&self, name: &str) -> bool {
        self.reports.iter().any(|(n, _)| *n == name)
    }

    /// Exit with a redirect (known report, wrong tier) or unknown-report
    /// error for `name`.
    fn reject(&self, name: &str, origin: &str) -> ! {
        if let Some(desc) = reports::describe(name) {
            eprintln!(
                "error: report '{name}' ({desc}) {origin} is not served by {} — run {} instead",
                self.bin, self.other_bin
            );
        } else {
            eprintln!(
                "error: unknown report '{name}' {origin}. available in {}: {}",
                self.bin,
                self.reports
                    .iter()
                    .map(|(n, _)| *n)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        process::exit(2);
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

/// Load the portfolio and run the requested reports for one tier.
///
/// Both the explicit `--report` values and the config's report list are
/// tier-owned: a name the tier doesn't serve errors (with a pointer to
/// the sibling binary when the name belongs there). Exits the process on
/// error, matching the previous single-binary behavior.
pub fn run(
    tier: &Tier,
    config_path: &Path,
    market: Option<&Path>,
    as_of: Option<Date>,
    requested: &[String],
) -> ! {
    for name in requested {
        if !tier.serves(name) {
            tier.reject(name, "");
        }
    }

    let portfolio = (tier.load)(config_path, market, as_of).unwrap_or_else(|e| {
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

    let report_names: Vec<&str> = if requested.is_empty() {
        if portfolio.reports.is_empty() {
            tier.fallback.to_vec()
        } else {
            for name in &portfolio.reports {
                if !tier.serves(name) {
                    tier.reject(name, "(from config)");
                }
            }
            portfolio.reports.iter().map(String::as_str).collect()
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

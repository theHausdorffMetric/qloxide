use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;

use crate::core;
use crate::dates::Date;
use crate::instruments::{ClearingStatus, FinancialInstrument};
use crate::market_data::{MarketData, MarketHistory};
use crate::trades::Deal;

/// TOML configuration pointing to JSON data files.
///
/// Book and risk are separate configs (architecture §9, resolving §8.3):
/// `book.toml` is the contract of record and rejects unknown keys — a risk
/// key in a book config is a schema error, so "book data is vol-free" is
/// enforced structurally. `risk.toml` is flat and independent: it lists
/// the same data files directly and adds risk-tier enrichment. Sharing
/// happens at the data files, not the configs.
///
/// Paths are resolved relative to the directory containing the config
/// file. `market` names the one-file market history (`{source, generator,
/// skipped_days, days: [...]}`); it is optional — the static reports
/// (instruments, deals, positions) run without it.
///
/// ```toml
/// instruments = ["instruments.json"]
/// deals = ["deals.json"]
/// market = "market.json"
/// reports = ["instruments", "deals", "positions", "pnl"]
/// ```
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BookConfig {
    pub instruments: Vec<PathBuf>,
    /// Settlement-index registry files. Every `Future.underlying` must
    /// resolve here — a book with futures and no indices is a config error.
    #[serde(default)]
    pub indices: Vec<PathBuf>,
    pub deals: Vec<PathBuf>,
    /// The book's market history file.
    #[serde(default)]
    pub market: Option<PathBuf>,
    /// Date-scoped waivers: completeness findings inside a waiver window
    /// downgrade from integrity errors to warnings.
    #[serde(default)]
    pub waivers: Vec<Waiver>,
    /// Per-book proxy marks: uncleared lookalike id → cleared twin whose
    /// settle serves as the official mark (architecture.md §5). An explicit
    /// valuation-policy decision, committed in the book's config:
    ///
    /// ```toml
    /// [proxy_marks]
    /// "BIL-BRN-U26" = "ICE-B-U26"
    /// ```
    #[serde(default)]
    pub proxy_marks: crate::portfolio::ProxyMarks,
    #[serde(default)]
    pub reports: Vec<String>,
}

/// The risk tier's own config (`risk.toml`): the same data files as the
/// book, listed directly, plus risk-only enrichment. Policy tables
/// (proxy_marks, waivers) are duplicated from the book config where the
/// risk report needs them — an accepted trade-off; the risk report prints
/// the policy it ran under so divergence is visible.
///
/// ```toml
/// instruments = ["instruments.json"]
/// deals = ["deals.json"]
/// market = "market.json"
/// vol_data = ["vols.json"]
/// reports = ["risk"]
/// ```
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskConfig {
    pub instruments: Vec<PathBuf>,
    /// Settlement-index registry files (same semantics as the book tier).
    #[serde(default)]
    pub indices: Vec<PathBuf>,
    pub deals: Vec<PathBuf>,
    /// The book's market history file.
    #[serde(default)]
    pub market: Option<PathBuf>,
    /// Risk-tier vol histories, merged day-wise into the market history —
    /// the P&L path never reads these (I2); surfaces an uncleared position
    /// needs for *marking* must live in the market file itself.
    #[serde(default)]
    pub vol_data: Vec<PathBuf>,
    #[serde(default)]
    pub waivers: Vec<Waiver>,
    #[serde(default)]
    pub proxy_marks: crate::portfolio::ProxyMarks,
    #[serde(default)]
    pub reports: Vec<String>,
}

/// The tier-independent loading recipe a config resolves to.
struct LoadSpec {
    instruments: Vec<PathBuf>,
    indices: Vec<PathBuf>,
    deals: Vec<PathBuf>,
    market: Option<PathBuf>,
    vol_data: Vec<PathBuf>,
    waivers: Vec<Waiver>,
    proxy_marks: crate::portfolio::ProxyMarks,
    reports: Vec<String>,
    risk_tier: bool,
}

/// A date-scoped waiver for known series holes (e.g. an upstream data gap).
/// Committed in the book's config — part of its definition, not a CLI flag.
///
/// ```toml
/// [[waivers]]
/// from = "2020-01-01"
/// through = "2020-09-30"
/// reason = "icedat oil-futures history gap"
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct Waiver {
    pub from: Date,
    pub through: Date,
    pub reason: String,
}

impl Waiver {
    fn covers(&self, date: Date) -> bool {
        self.from <= date && date <= self.through
    }
}

/// Assembled data ready for pricing, returned by [`load`].
#[derive(Debug)]
pub struct Portfolio {
    pub instruments: HashMap<String, Arc<dyn FinancialInstrument>>,
    pub deals: Vec<Deal>,
    /// The valuation-day record selected from the history (the last day,
    /// or `--as-of`). `None` when the config names no market file —
    /// static reports still run; valuation errors.
    pub market_data: Option<MarketData>,
    /// The full market history behind the book, when configured. For the
    /// risk tier, vol_data enrichment is already merged day-wise.
    pub history: Option<MarketHistory>,
    pub warnings: Vec<String>,
    /// Unwaived completeness failures. Valuation reports refuse while any
    /// are present; static listings still render (diagnosability).
    pub integrity_errors: Vec<String>,
    /// Validated proxy-mark policy (see [`BookConfig::proxy_marks`]).
    pub proxy_marks: crate::portfolio::ProxyMarks,
    /// The settlement-index registry (validated: refs resolve, no cycles,
    /// every future's underlying resolves, Average windows end on expiry).
    pub indices: crate::reference_data::SettlementIndexRegistry,
    pub reports: Vec<String>,
}

/// Load a book config and assemble a [`Portfolio`] valued at the
/// history's last day. See [`load_book`].
pub fn load(config_path: &Path) -> core::Result<Portfolio> {
    load_book(config_path, None)
}

/// Load a `book.toml` (strict schema — risk keys are an error) and
/// assemble a [`Portfolio`]: valuation day = the history's last day, or
/// `as_of` (what-if against any contained day). Surfaces in the market
/// data that no position needs draw a warning (architecture §9 — book
/// data is vol-free).
pub fn load_book(config_path: &Path, as_of: Option<Date>) -> core::Result<Portfolio> {
    let (config_dir, toml_str) = read_config(config_path)?;
    let config: BookConfig = toml::from_str(&toml_str).map_err(|e| {
        core::Error::Config(format!("invalid config {}: {}", config_path.display(), e))
    })?;
    let spec = LoadSpec {
        instruments: config.instruments,
        indices: config.indices,
        deals: config.deals,
        market: config.market,
        vol_data: Vec::new(),
        waivers: config.waivers,
        proxy_marks: config.proxy_marks,
        reports: config.reports,
        risk_tier: false,
    };
    load_impl(&config_dir, spec, None, as_of)
}

/// Load a `risk.toml` (risk-tier semantics, architecture §9): the
/// config's `vol_data` histories are merged day-wise into the market
/// history — unless a `--market` override replaces the market wholesale
/// (scenario files are self-contained one-day histories, resolved
/// relative to the caller's cwd) — and the book tier's unneeded-surface
/// warning is skipped: risk surfaces are beyond marking needs by
/// definition.
pub fn load_risk(
    config_path: &Path,
    market_override: Option<&Path>,
    as_of: Option<Date>,
) -> core::Result<Portfolio> {
    let (config_dir, toml_str) = read_config(config_path)?;
    let config: RiskConfig = toml::from_str(&toml_str).map_err(|e| {
        core::Error::Config(format!("invalid config {}: {}", config_path.display(), e))
    })?;
    let spec = LoadSpec {
        instruments: config.instruments,
        indices: config.indices,
        deals: config.deals,
        market: config.market,
        vol_data: config.vol_data,
        waivers: config.waivers,
        proxy_marks: config.proxy_marks,
        reports: config.reports,
        risk_tier: true,
    };
    load_impl(&config_dir, spec, market_override, as_of)
}

fn read_config(config_path: &Path) -> core::Result<(PathBuf, String)> {
    let config_dir = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let toml_str = std::fs::read_to_string(config_path).map_err(|e| {
        core::Error::Config(format!("cannot read {}: {}", config_path.display(), e))
    })?;
    Ok((config_dir, toml_str))
}

fn load_impl(
    config_dir: &Path,
    config: LoadSpec,
    market_override: Option<&Path>,
    as_of: Option<Date>,
) -> core::Result<Portfolio> {
    let risk_tier = config.risk_tier;

    // Load instruments
    let mut instruments: HashMap<String, Arc<dyn FinancialInstrument>> = HashMap::new();
    for rel_path in &config.instruments {
        let path = config_dir.join(rel_path);
        let json = read_json_file(&path)?;
        let loaded = deserialize_instruments(&json, &path)?;
        for inst in loaded {
            let id = inst.id().to_string();
            if instruments.contains_key(&id) {
                return Err(core::Error::Config(format!(
                    "duplicate instrument ID '{}'",
                    id
                )));
            }
            instruments.insert(id, inst);
        }
    }

    // Load the settlement-index registry (design note v3): duplicate ids
    // error per-file at insert; graph validation (dangling refs, cycles)
    // runs on the assembled whole.
    let mut all_indices: Vec<crate::reference_data::SettlementIndex> = Vec::new();
    for rel_path in &config.indices {
        let path = config_dir.join(rel_path);
        let json = read_json_file(&path)?;
        let loaded: Vec<crate::reference_data::SettlementIndex> = serde_json::from_str(&json)
            .map_err(|e| {
                core::Error::Config(format!("invalid indices file {}: {}", path.display(), e))
            })?;
        all_indices.extend(loaded);
    }
    let indices = crate::reference_data::SettlementIndexRegistry::from_indices(all_indices)?;
    indices.validate()?;

    // Every future's underlying must resolve (no opaque-leaf-by-absence),
    // and an Average determination period must end on the contract's
    // expiry — both are the last working day of the determination month,
    // so a mismatch means the contract points at the wrong period entry.
    {
        let mut ids: Vec<&String> = instruments.keys().collect();
        ids.sort();
        for id in ids {
            let Some(fut) = instruments[id]
                .as_any()
                .downcast_ref::<crate::instruments::Future>()
            else {
                continue;
            };
            let Some(index) = indices.get(&fut.underlying) else {
                return Err(core::Error::Config(format!(
                    "future '{}' references unknown settlement index '{}'",
                    id, fut.underlying,
                )));
            };
            if let crate::reference_data::IndexRule::Average { window, .. } = &index.rule
                && window.1 != fut.expiry
            {
                return Err(core::Error::Config(format!(
                    "future '{}': determination window of '{}' ends {} but expiry is {} \
                     — contract points at the wrong period entry",
                    id, fut.underlying, window.1, fut.expiry,
                )));
            }
        }
    }

    // Load deals
    let mut deals: Vec<Deal> = Vec::new();
    let mut deal_ids: HashMap<String, bool> = HashMap::new();
    for rel_path in &config.deals {
        let path = config_dir.join(rel_path);
        let json = read_json_file(&path)?;
        let loaded = deserialize_deals(&json, &path)?;
        for deal in loaded {
            if deal_ids.contains_key(&deal.id) {
                return Err(core::Error::Config(format!(
                    "duplicate deal ID '{}'",
                    deal.id
                )));
            }
            deal_ids.insert(deal.id.clone(), true);
            deals.push(deal);
        }
    }

    // Load the market history. An override replaces the config's market
    // wholesale and resolves relative to the cwd, not the config file
    // (the scenario entry point: a self-contained one-day history).
    let market_path: Option<PathBuf> = match market_override {
        Some(p) => Some(p.to_path_buf()),
        None => config.market.as_ref().map(|rel| config_dir.join(rel)),
    };
    let mut history: Option<MarketHistory> = None;
    if let Some(path) = &market_path {
        history = Some(load_history(path, "market data")?);
    }
    // Risk-tier enrichment: merge the vol_data histories day-wise
    // (skipped under a --market override — scenario files are
    // self-contained).
    if risk_tier && market_override.is_none() {
        for rel_path in &config.vol_data {
            let path = config_dir.join(rel_path);
            let vols = load_history(&path, "vol data")?;
            match &mut history {
                None => {
                    return Err(core::Error::Config(format!(
                        "vol_data {} without a market file — surfaces enrich a market, they aren't one",
                        path.display()
                    )));
                }
                Some(existing) => existing.merge_days(vols).map_err(|e| {
                    core::Error::Config(format!("vol data {}: {}", path.display(), e))
                })?,
            }
        }
    }
    if let Some(h) = &history {
        h.validate()
            .map_err(|e| core::Error::Config(format!("market data: {e}")))?;
    }

    // Select the valuation day: the last record, or --as-of. The record
    // inherits the header's provenance stamps so reports can label their
    // source without per-day duplication.
    let market_data: Option<MarketData> = match &history {
        None => {
            if let Some(date) = as_of {
                return Err(core::Error::Config(format!(
                    "--as-of {date} without a market file"
                )));
            }
            None
        }
        Some(h) => {
            let day = match as_of {
                Some(date) => h.day(date)?,
                None => h.last()?,
            };
            let mut md = day.clone();
            if md.source().is_none()
                && let Some(s) = h.source()
            {
                md.set_source(s);
            }
            if md.generator().is_none()
                && let Some(g) = h.generator()
            {
                md.set_generator(g);
            }
            Some(md)
        }
    };

    // Consistency checks
    let mut warnings: Vec<String> = Vec::new();

    // Currencies are embedded by value in each instrument's JSON; two
    // instruments declaring the same currency id with different
    // conventions is a silent-mismatch hazard.
    let mut currencies: HashMap<String, (String, crate::reference_data::Currency)> = HashMap::new();
    let mut sorted_ids: Vec<&String> = instruments.keys().collect();
    sorted_ids.sort(); // deterministic "first seen" for stable warnings
    for id in sorted_ids {
        let inst = &instruments[id];
        let ccy = inst.currency();
        match currencies.get(&ccy.id) {
            None => {
                currencies.insert(ccy.id.clone(), (id.clone(), ccy.clone()));
            }
            Some((first_inst, first_ccy)) if first_ccy != ccy => {
                warnings.push(format!(
                    "currency '{}' defined inconsistently: instrument '{}' disagrees with '{}'",
                    ccy.id, id, first_inst,
                ));
            }
            Some(_) => {}
        }

        // Dollar-terms P&L inherits an option's contract size from its
        // underlying future; anything else silently defaults to 1
        // (`portfolio::contract_size`) — surface that at load. Warn now,
        // error later (settlement-index design note, validation).
        if let Some(opt) = inst
            .as_any()
            .downcast_ref::<crate::instruments::EuropeanOption>()
        {
            match instruments.get(&opt.underlying) {
                None => warnings.push(format!(
                    "option '{}': unknown underlying '{}'",
                    id, opt.underlying,
                )),
                Some(u)
                    if u.as_any()
                        .downcast_ref::<crate::instruments::Future>()
                        .is_none() =>
                {
                    warnings.push(format!(
                        "option '{}': underlying '{}' is not a future — contract_size defaults to 1",
                        id, opt.underlying,
                    ));
                }
                Some(_) => {}
            }
        }
    }

    for deal in &deals {
        if !instruments.contains_key(&deal.instrument_id) {
            return Err(core::Error::Config(format!(
                "deal '{}' references unknown instrument '{}'",
                deal.id, deal.instrument_id,
            )));
        }
    }

    // Proxy-mark policy is part of the book's definition — a broken mapping
    // is a config error, not a warning.
    for (id, twin) in &config.proxy_marks {
        let Some(inst) = instruments.get(id) else {
            return Err(core::Error::Config(format!(
                "proxy_marks: unknown instrument '{id}'"
            )));
        };
        if !matches!(inst.clearing(), Some(ClearingStatus::Uncleared)) {
            return Err(core::Error::Config(format!(
                "proxy_marks: '{id}' is not uncleared — it marks at its own settle"
            )));
        }
        let Some(twin_inst) = instruments.get(twin) else {
            return Err(core::Error::Config(format!(
                "proxy_marks: '{id}' proxies unknown instrument '{twin}'"
            )));
        };
        if !matches!(twin_inst.clearing(), Some(ClearingStatus::Cleared)) {
            return Err(core::Error::Config(format!(
                "proxy_marks: twin '{twin}' of '{id}' is not cleared — a proxy mark borrows an official settle"
            )));
        }
    }

    // Market-dependent checks only apply when market data is loaded; a
    // static (listing-only) run has nothing to check marks against. The
    // *requirement* an instrument places on the market data is dispatched
    // on classification (architecture.md §5): cleared → its settle;
    // uncleared+proxy → the twin's settle; uncleared model-marked options →
    // a statically arbitrage-free vol surface. A failed requirement on a
    // *dealt* instrument blocks pnl (integrity error, waivable by date);
    // on an undealt one it stays an advisory warning.
    let mut integrity_errors: Vec<String> = Vec::new();
    if let Some(market_data) = &market_data {
        let waived_now = config
            .waivers
            .iter()
            .any(|w| w.covers(market_data.valuation_date()));
        let dealt: std::collections::BTreeSet<&str> =
            deals.iter().map(|d| d.instrument_id.as_str()).collect();
        // Underlyings whose surface is a *marking* requirement (unexpired,
        // uncleared, unproxied options) — everything beyond this set is
        // risk-tier data that doesn't belong in book market data (§9).
        let mut needed_surfaces: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();

        for (id, inst) in &instruments {
            let ccy = inst.currency().id.clone();
            if market_data.discount_curve(&ccy).is_err() {
                warnings.push(format!(
                    "instrument '{}': no discount curve for currency '{}'",
                    id, ccy,
                ));
            }
            let expired = inst
                .maturity()
                .is_some_and(|m| market_data.valuation_date() > m);
            let is_dealt = dealt.contains(id.as_str());
            let cleared = matches!(inst.clearing(), Some(ClearingStatus::Cleared));

            // Settle-primary marking: a cleared instrument's official mark is
            // its settlement price — flag its absence regardless of expiry.
            // A proxied uncleared instrument places the same requirement on
            // its twin's settle.
            if cleared && market_data.settlement_price(id).is_err() {
                require(
                    is_dealt,
                    waived_now,
                    format!("instrument '{id}': cleared but no settlement price (official mark)"),
                    &mut warnings,
                    &mut integrity_errors,
                );
            }
            if let Some(twin) = config.proxy_marks.get(id.as_str())
                && market_data.settlement_price(twin).is_err()
            {
                require(
                    is_dealt,
                    waived_now,
                    format!(
                        "instrument '{id}': proxy twin '{twin}' has no settlement price (official mark)"
                    ),
                    &mut warnings,
                    &mut integrity_errors,
                );
            }

            // Options price via the model, not a quoted market price: check
            // their actual inputs (underlying instrument + vol surface)
            // instead. For an uncleared, unproxied option the surface is a
            // marking requirement, not a risk-side nicety.
            if let Some(opt) = inst
                .as_any()
                .downcast_ref::<crate::instruments::EuropeanOption>()
            {
                let model_marked = matches!(inst.clearing(), Some(ClearingStatus::Uncleared))
                    && !config.proxy_marks.contains_key(id.as_str());
                if !expired && model_marked {
                    needed_surfaces.insert(opt.underlying.clone());
                }
                if !expired && !market_data.has_vol_surface(&opt.underlying) {
                    if model_marked {
                        require(
                            is_dealt,
                            waived_now,
                            format!(
                                "option '{id}': uncleared and model-marked, but no vol surface for underlying '{}'",
                                opt.underlying,
                            ),
                            &mut warnings,
                            &mut integrity_errors,
                        );
                    } else if risk_tier {
                        // For a cleared/proxied option the surface is a
                        // risk-side input (greeks), not a marking need —
                        // its absence is only worth noting on the risk
                        // tier; vol-free book data is the §9 contract.
                        warnings.push(format!(
                            "option '{}': no vol surface for underlying '{}'",
                            id, opt.underlying,
                        ));
                    }
                }
                continue;
            }

            if expired {
                if !cleared && market_data.settlement_price(id).is_err() {
                    warnings.push(format!(
                        "instrument '{}': expired but no settlement price",
                        id,
                    ));
                }
            } else if market_data.market_price(id).is_err() {
                // The quote also feeds the model path (options read the
                // underlying's market price as the forward).
                warnings.push(format!("instrument '{}': no market price", id,));
            }
        }

        // No-arb gate: a surface that model-marks an uncleared, dealt,
        // unexpired position produces books-grade P&L — it must be free of
        // static arbitrage (§5). Findings are integrity errors; the alarm
        // is the deliverable, repair is an upstream policy decision.
        let mut gated: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for (id, inst) in &instruments {
            if !dealt.contains(id.as_str())
                || !matches!(inst.clearing(), Some(ClearingStatus::Uncleared))
                || config.proxy_marks.contains_key(id.as_str())
            {
                continue;
            }
            let Some(opt) = inst
                .as_any()
                .downcast_ref::<crate::instruments::EuropeanOption>()
            else {
                continue;
            };
            if inst
                .maturity()
                .is_some_and(|m| market_data.valuation_date() > m)
                || !gated.insert(opt.underlying.as_str())
            {
                continue;
            }
            if let Ok(surface) = market_data.vol_surface(&opt.underlying) {
                for finding in crate::pricing::no_arb::surface_no_arb(surface) {
                    require(
                        true,
                        waived_now,
                        format!(
                            "vol surface '{}' (books-grade, model-marks uncleared '{}'): {}",
                            opt.underlying, id, finding,
                        ),
                        &mut warnings,
                        &mut integrity_errors,
                    );
                }
            }
        }

        // The inverse of the marking requirement (§9): book data should be
        // vol-free beyond what model-marking needs — the risk tier loads
        // its surfaces from vol_data instead.
        if !risk_tier {
            let unneeded: Vec<&str> = market_data
                .vol_surface_ids()
                .into_iter()
                .filter(|id| !needed_surfaces.contains(*id))
                .collect();
            if !unneeded.is_empty() {
                warnings.push(format!(
                    "market data carries {} vol surface(s) no position needs ({}) — \
                     book data should be vol-free; move them to a vol_data file (risk tier)",
                    unneeded.len(),
                    unneeded.join(", ")
                ));
            }
        }
    }

    // Series settle-completeness: for every cleared, dealt instrument,
    // each trading day in [earliest deal date, eval] must carry a settle.
    // The history claims to run from inception (§9), so this always runs
    // when a market file is present. Unwaived findings are integrity
    // errors — pnl refuses on them.
    if let Some(h) = &history {
        let eval = market_data
            .as_ref()
            .map(|md| md.valuation_date())
            .or_else(|| h.last_day());
        if let Some(eval) = eval {
            check_series_completeness(
                h,
                &instruments,
                &deals,
                eval,
                &config.waivers,
                &mut warnings,
                &mut integrity_errors,
            );
        }
    }

    Ok(Portfolio {
        instruments,
        deals,
        market_data,
        history,
        warnings,
        integrity_errors,
        proxy_marks: config.proxy_marks,
        indices,
        reports: config.reports,
    })
}

/// Read and parse a one-file market history. A legacy single-day file
/// (top-level `valuation_date`) gets a targeted migration error.
fn load_history(path: &Path, what: &str) -> core::Result<MarketHistory> {
    let json = read_json_file(path)?;
    match serde_json::from_str::<MarketHistory>(&json) {
        Ok(h) => Ok(h),
        Err(e) => {
            let legacy = serde_json::from_str::<serde_json::Value>(&json)
                .ok()
                .is_some_and(|v| v.get("valuation_date").is_some());
            if legacy {
                Err(core::Error::Config(format!(
                    "{what} {} is a legacy single-day file — market files are histories \
                     now: wrap the record in {{\"days\": [ … ]}} (architecture §9)",
                    path.display()
                )))
            } else {
                Err(core::Error::Config(format!(
                    "invalid {what} {}: {}",
                    path.display(),
                    e
                )))
            }
        }
    }
}

/// Route a completeness finding by severity: a dealt instrument's failed
/// marking requirement is an integrity error (pnl refuses) unless waived;
/// undealt findings are advisory warnings.
fn require(
    dealt: bool,
    waived: bool,
    finding: String,
    warnings: &mut Vec<String>,
    errors: &mut Vec<String>,
) {
    if dealt && !waived {
        errors.push(finding);
    } else if dealt {
        warnings.push(format!("waived: {finding}"));
    } else {
        warnings.push(finding);
    }
}

/// Kinds of per-day completeness findings.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Gap {
    /// Trading day in the series, but no settle recorded for the instrument.
    MissingSettle,
    /// Calendar day the series does not account for at all.
    Uncovered,
}

/// Walk [inception, eval] per cleared dealt instrument, collapse findings
/// into contiguous runs (same kind, same waived-status), and emit each run
/// as one warning (waived) or one integrity error (not).
#[allow(clippy::too_many_arguments)]
fn check_series_completeness(
    store: &MarketHistory,
    instruments: &HashMap<String, Arc<dyn FinancialInstrument>>,
    deals: &[Deal],
    eval: Date,
    waivers: &[Waiver],
    warnings: &mut Vec<String>,
    errors: &mut Vec<String>,
) {
    // Earliest deal date per cleared instrument (uncleared marks to model —
    // exempt; undealt instruments have no M2M history to demand).
    let mut inception: BTreeMap<&str, Date> = BTreeMap::new();
    for deal in deals {
        let Some(inst) = instruments.get(&deal.instrument_id) else {
            continue;
        };
        if !matches!(inst.clearing(), Some(ClearingStatus::Cleared)) {
            continue;
        }
        let d = deal.timestamp.date();
        inception
            .entry(deal.instrument_id.as_str())
            .and_modify(|e| {
                if d < *e {
                    *e = d;
                }
            })
            .or_insert(d);
    }

    for (id, first) in inception {
        // (kind, waived, from, to) runs of problematic days
        let mut runs: Vec<(Gap, bool, Date, Date)> = Vec::new();
        let mut date = first;
        while date <= eval {
            let gap = if store.is_trading_day(date) {
                (!store.has_settle(date, id)).then_some(Gap::MissingSettle)
            } else if store.is_known(date) {
                None // explicitly non-trading
            } else {
                Some(Gap::Uncovered)
            };
            if let Some(kind) = gap {
                let waived = waivers.iter().any(|w| w.covers(date));
                match runs.last_mut() {
                    Some((k, wv, _, to)) if *k == kind && *wv == waived && *to + 1 == date => {
                        *to = date;
                    }
                    _ => runs.push((kind, waived, date, date)),
                }
            }
            date = date + 1;
        }

        for (kind, waived, from, to) in runs {
            let span = if from == to {
                from.to_string()
            } else {
                format!("{from}..{to}")
            };
            let what = match kind {
                Gap::MissingSettle => {
                    format!("cleared instrument '{id}': no settlement in series on {span}")
                }
                Gap::Uncovered => {
                    format!("cleared instrument '{id}': series does not cover {span}")
                }
            };
            if waived {
                let reason = waivers
                    .iter()
                    .find(|w| w.covers(from))
                    .map(|w| w.reason.as_str())
                    .unwrap_or("waived");
                warnings.push(format!("waived ({reason}): {what}"));
            } else {
                errors.push(what);
            }
        }
    }
}

fn read_json_file(path: &Path) -> core::Result<String> {
    std::fs::read_to_string(path)
        .map_err(|e| core::Error::Config(format!("cannot read {}: {}", path.display(), e)))
}

/// Deserialize a JSON string as either a single instrument or an array.
///
/// Dispatches on the first non-whitespace byte rather than try-and-fall-back,
/// so an error inside an array element is reported as that element's error
/// instead of a misleading "expected a single object" failure.
fn deserialize_instruments(
    json: &str,
    path: &Path,
) -> core::Result<Vec<Arc<dyn FinancialInstrument>>> {
    if json.trim_start().starts_with('[') {
        serde_json::from_str::<Vec<Arc<dyn FinancialInstrument>>>(json).map_err(|e| {
            core::Error::Config(format!("invalid instrument JSON {}: {}", path.display(), e))
        })
    } else {
        let single: Arc<dyn FinancialInstrument> = serde_json::from_str(json).map_err(|e| {
            core::Error::Config(format!("invalid instrument JSON {}: {}", path.display(), e))
        })?;
        Ok(vec![single])
    }
}

/// Deserialize a JSON string as either a single deal or an array.
/// Same first-byte dispatch as [`deserialize_instruments`].
fn deserialize_deals(json: &str, path: &Path) -> core::Result<Vec<Deal>> {
    if json.trim_start().starts_with('[') {
        serde_json::from_str::<Vec<Deal>>(json).map_err(|e| {
            core::Error::Config(format!("invalid deal JSON {}: {}", path.display(), e))
        })
    } else {
        let single: Deal = serde_json::from_str(json).map_err(|e| {
            core::Error::Config(format!("invalid deal JSON {}: {}", path.display(), e))
        })?;
        Ok(vec![single])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// The standard fixture span: deal inception (2026-03-02) through eval
    /// (2026-03-07), one record per day so completeness stays clean.
    const FIXTURE_DAYS: &[&str] = &[
        "2026-03-02",
        "2026-03-03",
        "2026-03-04",
        "2026-03-05",
        "2026-03-06",
        "2026-03-07",
    ];

    /// Wrap per-day record bodies into the one-file history envelope.
    fn history_json(days: &[&str], body: impl Fn(&str) -> String) -> String {
        let records: Vec<String> = days.iter().map(|d| body(d)).collect();
        format!("{{ \"days\": [\n{}\n] }}", records.join(",\n"))
    }

    /// The standard single-future day record.
    fn future_day(date: &str) -> String {
        format!(
            r#"{{
  "valuation_date": "{date}",
  "as_of": "{date}T14:00:00Z",
  "market_prices": {{"ICE-B-K26": 72.45}},
  "settlement_prices": {{"ICE-B-K26": 72.45}},
  "discount_curves": {{
    "USD": {{
      "base_date": "{date}",
      "day_count": "Act360",
      "pillars": [["2026-06-07", 0.043]]
    }}
  }}
}}"#
        )
    }

    fn write_test_files(dir: &Path) {
        // Config
        fs::write(
            dir.join("pricing.toml"),
            r#"
instruments = ["instruments.json"]
indices = ["indices.json"]
deals = ["deals.json"]
market = "market.json"
"#,
        )
        .unwrap();

        // Settlement-index registry
        fs::write(
            dir.join("indices.json"),
            r#"[{
  "id": "ICE-BRENT-INDEX",
  "display_name": "Brent",
  "rule": { "Published": { "source": "ICE" } },
  "calendar": "ICEUK"
}]"#,
        )
        .unwrap();

        // Single instrument
        fs::write(
            dir.join("instruments.json"),
            r#"{
  "type": "Future",
  "id": "ICE-B-K26",
  "underlying": "ICE-BRENT-INDEX",
  "currency": {"id": "USD", "settlement": "Null", "day_count": "Act360"},
  "settlement": {"venue": "ICE", "session": "SETTLE", "time": "19:30", "timezone": "Europe/London", "payment_lag": "Null"},
  "clearing": "cleared",
  "expiry": "2026-03-31",
  "contract_size": "1000",
  "tick_size": "0.01"
}"#,
        )
        .unwrap();

        // Single deal
        fs::write(
            dir.join("deals.json"),
            r#"{
  "id": "DEAL-001",
  "instrument_id": "ICE-B-K26",
  "direction": "Buy",
  "quantity": "5",
  "price": "71.80",
  "timestamp": "2026-03-02T10:14:32Z",
  "counterparty": "ICE_CLEAR",
  "venue": "ICE"
}"#,
        )
        .unwrap();

        // Market history: inception through eval, complete settles.
        fs::write(
            dir.join("market.json"),
            history_json(FIXTURE_DAYS, future_day),
        )
        .unwrap();
    }

    #[test]
    fn load_single_object_files() {
        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());

        let portfolio = load(&dir.path().join("pricing.toml")).unwrap();
        assert_eq!(portfolio.instruments.len(), 1);
        assert!(portfolio.instruments.contains_key("ICE-B-K26"));
        assert_eq!(portfolio.deals.len(), 1);
        assert_eq!(portfolio.deals[0].instrument_id, "ICE-B-K26");
        assert!(portfolio.warnings.is_empty());
    }

    #[test]
    fn loads_without_market_data() {
        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());

        // No market_data key: static reports run, valuation refuses.
        fs::write(
            dir.path().join("pricing.toml"),
            "instruments = [\"instruments.json\"]\nindices = [\"indices.json\"]\ndeals = [\"deals.json\"]\n",
        )
        .unwrap();

        let portfolio = load(&dir.path().join("pricing.toml")).unwrap();
        assert!(portfolio.market_data.is_none());
        // Market-dependent checks are skipped — no spurious warnings.
        assert!(portfolio.warnings.is_empty(), "{:?}", portfolio.warnings);

        let listing = crate::reports::run("instruments", &portfolio).unwrap();
        assert!(listing.contains("ICE-B-K26"), "{listing}");
        assert!(listing.contains("Future"), "{listing}");

        let err = crate::reports::run("pnl", &portfolio)
            .unwrap_err()
            .to_string();
        assert!(err.contains("requires market_data"), "{err}");
    }

    #[test]
    fn dangling_future_underlying_errors() {
        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());

        // Empty registry: no opaque-leaf-by-absence — this is an error.
        fs::write(dir.path().join("indices.json"), "[]").unwrap();

        let err = load(&dir.path().join("pricing.toml"))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("unknown settlement index 'ICE-BRENT-INDEX'"),
            "{err}"
        );
    }

    #[test]
    fn average_window_must_end_on_expiry() {
        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());

        // Fixture future expires 2026-03-31; point it at a period entry
        // whose window ends a day earlier — the wrong month binding.
        fs::write(
            dir.path().join("indices.json"),
            r#"[
  {"id": "ICE-BRENT-1L", "rule": {"Published": {"source": "ICE"}}, "calendar": "ICEUK"},
  {"id": "ICE-BRENT-INDEX", "rule": {"Average": {"of": "ICE-BRENT-1L", "window": ["2026-03-01", "2026-03-30"]}}}
]"#,
        )
        .unwrap();

        let err = load(&dir.path().join("pricing.toml"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("wrong period entry"), "{err}");
    }

    /// Rewrite market.json so the 2026-03-04 record optionally lacks the
    /// instrument's settle — the completeness axis.
    fn write_series_market(dir: &Path, covered: bool) {
        let body = |date: &str| {
            if date == "2026-03-04" && !covered {
                future_day(date).replace(
                    r#""settlement_prices": {"ICE-B-K26": 72.45}"#,
                    r#""settlement_prices": {}"#,
                )
            } else {
                future_day(date)
            }
        };
        fs::write(dir.join("market.json"), history_json(FIXTURE_DAYS, body)).unwrap();
    }

    fn config_with_series(extra: &str) -> String {
        format!(
            "instruments = [\"instruments.json\"]\nindices = [\"indices.json\"]\ndeals = [\"deals.json\"]\n\
             market = \"market.json\"\n{extra}"
        )
    }

    #[test]
    fn series_completeness_clean() {
        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());
        write_series_market(dir.path(), true);
        fs::write(dir.path().join("pricing.toml"), config_with_series("")).unwrap();

        let portfolio = load(&dir.path().join("pricing.toml")).unwrap();
        assert!(
            portfolio.integrity_errors.is_empty(),
            "{:?}",
            portfolio.integrity_errors
        );
        assert!(portfolio.warnings.is_empty(), "{:?}", portfolio.warnings);
        assert!(portfolio.history.is_some());
    }

    #[test]
    fn series_missing_settle_is_integrity_error() {
        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());
        write_series_market(dir.path(), false);
        fs::write(dir.path().join("pricing.toml"), config_with_series("")).unwrap();

        let portfolio = load(&dir.path().join("pricing.toml")).unwrap();
        assert_eq!(portfolio.integrity_errors.len(), 1);
        assert!(
            portfolio.integrity_errors[0].contains("no settlement in series on 2026-03-04"),
            "{:?}",
            portfolio.integrity_errors
        );

        // Valuation refuses; static listings still render.
        let err = crate::reports::run("pnl", &portfolio)
            .unwrap_err()
            .to_string();
        assert!(err.contains("refused"), "{err}");
        assert!(crate::reports::run("instruments", &portfolio).is_ok());
    }

    #[test]
    fn series_waiver_downgrades_to_warning() {
        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());
        write_series_market(dir.path(), false);
        fs::write(
            dir.path().join("pricing.toml"),
            config_with_series(
                "[[waivers]]\nfrom = \"2026-03-04\"\nthrough = \"2026-03-04\"\nreason = \"test gap\"\n",
            ),
        )
        .unwrap();

        let portfolio = load(&dir.path().join("pricing.toml")).unwrap();
        assert!(
            portfolio.integrity_errors.is_empty(),
            "{:?}",
            portfolio.integrity_errors
        );
        assert!(
            portfolio
                .warnings
                .iter()
                .any(|w| w.contains("waived (test gap)")),
            "{:?}",
            portfolio.warnings
        );
        assert!(crate::reports::run("pnl", &portfolio).is_ok());
    }

    #[test]
    fn series_uncovered_days_collapse_to_one_run() {
        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());
        // History starts 03-04: 03-02..03-03 are unaccounted for.
        fs::write(
            dir.path().join("market.json"),
            history_json(
                &["2026-03-04", "2026-03-05", "2026-03-06", "2026-03-07"],
                future_day,
            ),
        )
        .unwrap();
        fs::write(dir.path().join("pricing.toml"), config_with_series("")).unwrap();

        let portfolio = load(&dir.path().join("pricing.toml")).unwrap();
        assert_eq!(
            portfolio.integrity_errors.len(),
            1,
            "{:?}",
            portfolio.integrity_errors
        );
        assert!(
            portfolio.integrity_errors[0].contains("does not cover 2026-03-02..2026-03-03"),
            "{:?}",
            portfolio.integrity_errors
        );
    }

    #[test]
    fn load_array_instruments() {
        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());

        // Overwrite with array of two instruments
        fs::write(
            dir.path().join("instruments.json"),
            r#"[
  {
    "type": "Future",
    "id": "ICE-B-K26",
    "underlying": "ICE-BRENT-INDEX",
    "currency": {"id": "USD", "settlement": "Null", "day_count": "Act360"},
    "settlement": {"venue": "ICE", "session": "SETTLE", "time": "19:30", "timezone": "Europe/London", "payment_lag": "Null"},
    "clearing": "cleared",
    "expiry": "2026-03-31",
    "contract_size": "1000",
    "tick_size": "0.01"
  },
  {
    "type": "Future",
    "id": "ICE-B-M26",
    "underlying": "ICE-BRENT-INDEX",
    "currency": {"id": "USD", "settlement": "Null", "day_count": "Act360"},
    "settlement": {"venue": "ICE", "session": "SETTLE", "time": "19:30", "timezone": "Europe/London", "payment_lag": "Null"},
    "clearing": "cleared",
    "expiry": "2026-05-29",
    "contract_size": "1000",
    "tick_size": "0.01"
  }
]"#,
        )
        .unwrap();

        let portfolio = load(&dir.path().join("pricing.toml")).unwrap();
        assert_eq!(portfolio.instruments.len(), 2);
        assert!(portfolio.instruments.contains_key("ICE-B-K26"));
        assert!(portfolio.instruments.contains_key("ICE-B-M26"));
    }

    #[test]
    fn duplicate_instrument_id_errors() {
        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());

        // Two files with same instrument ID
        fs::write(
            dir.path().join("pricing.toml"),
            r#"
instruments = ["instruments.json", "instruments2.json"]
indices = ["indices.json"]
deals = ["deals.json"]
market = "market.json"
"#,
        )
        .unwrap();
        fs::copy(
            dir.path().join("instruments.json"),
            dir.path().join("instruments2.json"),
        )
        .unwrap();

        let result = load(&dir.path().join("pricing.toml"));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("duplicate instrument"), "{}", err);
    }

    #[test]
    fn unknown_instrument_in_deal_errors() {
        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());

        fs::write(
            dir.path().join("deals.json"),
            r#"{
  "id": "D1",
  "instrument_id": "NONEXISTENT",
  "direction": "Buy",
  "quantity": "1",
  "price": "100",
  "timestamp": "2026-03-02T10:00:00Z",
  "counterparty": "X",
  "venue": "Y"
}"#,
        )
        .unwrap();

        let result = load(&dir.path().join("pricing.toml"));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("unknown instrument"), "{}", err);
    }

    #[test]
    fn missing_market_data_warns() {
        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());

        // A history whose days carry no market_prices, settles, or curves.
        fs::write(
            dir.path().join("market.json"),
            history_json(FIXTURE_DAYS, |date| {
                format!(
                    r#"{{
  "valuation_date": "{date}",
  "as_of": "{date}T14:00:00Z",
  "market_prices": {{}},
  "discount_curves": {{}}
}}"#
                )
            }),
        )
        .unwrap();

        let portfolio = load(&dir.path().join("pricing.toml")).unwrap();
        // no market price + no curve stay warnings; the missing settle on a
        // cleared *dealt* instrument escalates to an integrity error — once
        // at the eval day, once as the whole-span completeness run.
        assert_eq!(portfolio.warnings.len(), 2);
        assert_eq!(
            portfolio.integrity_errors.len(),
            2,
            "{:?}",
            portfolio.integrity_errors
        );
        assert!(
            portfolio
                .integrity_errors
                .iter()
                .any(|w| w.contains("cleared but no settlement price"))
        );
        assert!(
            portfolio
                .warnings
                .iter()
                .any(|w| w.contains("no market price"))
        );
        assert!(
            portfolio
                .warnings
                .iter()
                .any(|w| w.contains("no discount curve"))
        );
    }

    #[test]
    fn duplicate_deal_id_errors() {
        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());

        fs::write(
            dir.path().join("deals.json"),
            r#"[
  {"id": "D1", "instrument_id": "ICE-B-K26", "direction": "Buy", "quantity": "1", "price": "70", "timestamp": "2026-03-02T10:00:00Z", "counterparty": "X", "venue": "Y"},
  {"id": "D1", "instrument_id": "ICE-B-K26", "direction": "Sell", "quantity": "1", "price": "72", "timestamp": "2026-03-03T10:00:00Z", "counterparty": "X", "venue": "Y"}
]"#,
        )
        .unwrap();

        let result = load(&dir.path().join("pricing.toml"));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("duplicate deal"), "{}", err);
    }

    #[test]
    fn inconsistent_currency_definitions_warn() {
        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());

        // Second instrument declares USD with a different day count
        fs::write(
            dir.path().join("instruments.json"),
            r#"[
  {
    "type": "Future",
    "id": "ICE-B-K26",
    "underlying": "ICE-BRENT-INDEX",
    "currency": {"id": "USD", "settlement": "Null", "day_count": "Act360"},
    "settlement": {"venue": "ICE", "session": "SETTLE", "time": "19:30", "timezone": "Europe/London", "payment_lag": "Null"},
    "clearing": "cleared",
    "expiry": "2026-03-31",
    "contract_size": "1000",
    "tick_size": "0.01"
  },
  {
    "type": "Future",
    "id": "ICE-B-M26",
    "underlying": "ICE-BRENT-INDEX",
    "currency": {"id": "USD", "settlement": "Null", "day_count": "Act365Fixed"},
    "settlement": {"venue": "ICE", "session": "SETTLE", "time": "19:30", "timezone": "Europe/London", "payment_lag": "Null"},
    "clearing": "cleared",
    "expiry": "2026-05-29",
    "contract_size": "1000",
    "tick_size": "0.01"
  }
]"#,
        )
        .unwrap();

        let portfolio = load(&dir.path().join("pricing.toml")).unwrap();
        assert!(
            portfolio
                .warnings
                .iter()
                .any(|w| w.contains("defined inconsistently")),
            "expected currency inconsistency warning, got: {:?}",
            portfolio.warnings,
        );
    }

    #[test]
    fn option_warnings_check_underlying_and_vol_surface() {
        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());

        // Option referencing a missing underlying; no vol surface in market data
        fs::write(
            dir.path().join("instruments.json"),
            r#"[
  {
    "type": "Future",
    "id": "ICE-B-K26",
    "underlying": "ICE-BRENT-INDEX",
    "currency": {"id": "USD", "settlement": "Null", "day_count": "Act360"},
    "settlement": {"venue": "ICE", "session": "SETTLE", "time": "19:30", "timezone": "Europe/London", "payment_lag": "Null"},
    "clearing": "cleared",
    "expiry": "2026-03-31",
    "contract_size": "1000",
    "tick_size": "0.01"
  },
  {
    "type": "EuropeanOption",
    "id": "OPT-NO-VOL",
    "underlying": "ICE-B-K26",
    "credit_id": "ICE",
    "currency": {"id": "USD", "settlement": "Null", "day_count": "Act360"},
    "settlement": {"venue": "ICE", "session": "SETTLE", "time": "19:30", "timezone": "Europe/London", "payment_lag": "Null"},
    "clearing": "cleared",
    "expiry": "2026-03-27",
    "strike": "75",
    "put_or_call": "Call",
    "exercise_style": "European",
    "option_settlement": "Cash"
  },
  {
    "type": "EuropeanOption",
    "id": "OPT-NO-UNDERLYING",
    "underlying": "MISSING",
    "credit_id": "ICE",
    "currency": {"id": "USD", "settlement": "Null", "day_count": "Act360"},
    "settlement": {"venue": "ICE", "session": "SETTLE", "time": "19:30", "timezone": "Europe/London", "payment_lag": "Null"},
    "clearing": "cleared",
    "expiry": "2026-03-27",
    "strike": "75",
    "put_or_call": "Call",
    "exercise_style": "European",
    "option_settlement": "Cash"
  },
  {
    "type": "EuropeanOption",
    "id": "OPT-ON-OPT",
    "underlying": "OPT-NO-VOL",
    "credit_id": "ICE",
    "currency": {"id": "USD", "settlement": "Null", "day_count": "Act360"},
    "settlement": {"venue": "ICE", "session": "SETTLE", "time": "19:30", "timezone": "Europe/London", "payment_lag": "Null"},
    "clearing": "cleared",
    "expiry": "2026-03-27",
    "strike": "75",
    "put_or_call": "Call",
    "exercise_style": "European",
    "option_settlement": "Cash"
  }
]"#,
        )
        .unwrap();

        // Book tier: a cleared option's missing surface is the §9 contract
        // (vol-free book data), not a warning; the risk tier — where the
        // surface would feed greeks — still flags it.
        let portfolio = load(&dir.path().join("pricing.toml")).unwrap();
        assert!(
            !portfolio
                .warnings
                .iter()
                .any(|w| w.contains("OPT-NO-VOL") && w.contains("no vol surface")),
            "book tier must not warn on a cleared option's missing surface: {:?}",
            portfolio.warnings,
        );
        let risk_portfolio = load_risk(&dir.path().join("pricing.toml"), None, None).unwrap();
        assert!(
            risk_portfolio
                .warnings
                .iter()
                .any(|w| w.contains("OPT-NO-VOL") && w.contains("no vol surface")),
            "expected vol surface warning on the risk tier, got: {:?}",
            risk_portfolio.warnings,
        );
        assert!(
            portfolio
                .warnings
                .iter()
                .any(|w| w.contains("OPT-NO-UNDERLYING") && w.contains("unknown underlying")),
            "expected unknown underlying warning, got: {:?}",
            portfolio.warnings,
        );
        // An underlying that resolves but is not a future silently prices
        // dollar-terms P&L with contract_size = 1 — must draw a warning.
        assert!(
            portfolio
                .warnings
                .iter()
                .any(|w| w.contains("OPT-ON-OPT") && w.contains("contract_size defaults to 1")),
            "expected non-future underlying warning, got: {:?}",
            portfolio.warnings,
        );
        // Options must NOT trigger the generic "no market price" warning
        assert!(
            !portfolio
                .warnings
                .iter()
                .any(|w| w.contains("OPT-") && w.contains("no market price")),
            "options should not warn about market prices: {:?}",
            portfolio.warnings,
        );
    }

    #[test]
    fn malformed_array_element_reports_element_error() {
        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());

        // Array whose single element is missing required fields
        fs::write(
            dir.path().join("instruments.json"),
            r#"[{"type": "Future", "id": "BROKEN"}]"#,
        )
        .unwrap();

        let result = load(&dir.path().join("pricing.toml"));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        // The error must describe the element problem (missing field),
        // not a misleading "expected a single object" failure.
        assert!(err.contains("missing field"), "unhelpful error: {}", err);
    }

    #[test]
    fn missing_config_file_errors() {
        let result = load(Path::new("/nonexistent/pricing.toml"));
        assert!(result.is_err());
    }

    // ── classification-dispatched marking requirements (task 68) ─────

    /// A two-instrument book: cleared future + an option on it whose
    /// clearing status is the test's axis. Both have settles; the surface
    /// and any proxy/waiver config are the caller's choice.
    fn write_marking_book(
        dir: &Path,
        option_clearing: &str,
        vol_surfaces: Option<&str>,
        config_extra: &str,
    ) {
        fs::write(
            dir.join("pricing.toml"),
            format!(
                r#"
instruments = ["instruments.json"]
indices = ["indices.json"]
deals = ["deals.json"]
market = "market.json"
{config_extra}
"#
            ),
        )
        .unwrap();
        fs::write(
            dir.join("indices.json"),
            r#"[{"id": "ICE-BRENT-INDEX", "display_name": "Brent", "rule": {"Published": {"source": "ICE"}}, "calendar": "ICEUK"}]"#,
        )
        .unwrap();
        fs::write(
            dir.join("instruments.json"),
            format!(
                r#"[
  {{
    "type": "Future",
    "id": "ICE-B-K26",
    "underlying": "ICE-BRENT-INDEX",
    "currency": {{"id": "USD", "settlement": "Null", "day_count": "Act360"}},
    "settlement": {{"venue": "ICE", "session": "SETTLE", "time": "19:30", "timezone": "Europe/London", "payment_lag": "Null"}},
    "clearing": "cleared",
    "expiry": "2026-03-31",
    "contract_size": "1000",
    "tick_size": "0.01"
  }},
  {{
    "type": "EuropeanOption",
    "id": "OPT-BRN-75",
    "underlying": "ICE-B-K26",
    "credit_id": "ICE",
    "currency": {{"id": "USD", "settlement": "Null", "day_count": "Act360"}},
    "settlement": {{"venue": "ICE", "session": "SETTLE", "time": "19:30", "timezone": "Europe/London", "payment_lag": "Null"}},
    "clearing": "{option_clearing}",
    "expiry": "2026-03-27",
    "strike": "75",
    "put_or_call": "Call",
    "exercise_style": "European",
    "option_settlement": "Cash"
  }}
]"#
            ),
        )
        .unwrap();
        fs::write(
            dir.join("deals.json"),
            r#"[
  {"id": "D1", "instrument_id": "ICE-B-K26", "direction": "Buy", "quantity": "5", "price": "71.80", "timestamp": "2026-03-02T10:00:00Z", "counterparty": "X", "venue": "ICE"},
  {"id": "D2", "instrument_id": "OPT-BRN-75", "direction": "Buy", "quantity": "5", "price": "1.20", "timestamp": "2026-03-02T10:00:00Z", "counterparty": "X", "venue": "ICE"}
]"#,
        )
        .unwrap();
        let surfaces = vol_surfaces
            .map(|s| format!(",\n  \"vol_surfaces\": {s}"))
            .unwrap_or_default();
        let body = |date: &str| {
            format!(
                r#"{{
  "valuation_date": "{date}",
  "as_of": "{date}T14:00:00Z",
  "market_prices": {{"ICE-B-K26": 72.45}},
  "settlement_prices": {{"ICE-B-K26": 72.45, "OPT-BRN-75": 1.31}},
  "discount_curves": {{
    "USD": {{
      "base_date": "{date}",
      "day_count": "Act360",
      "pillars": [["2026-06-07", 0.043]]
    }}
  }}{surfaces}
}}"#
            )
        };
        fs::write(dir.join("market.json"), history_json(FIXTURE_DAYS, body)).unwrap();
    }

    const FLAT_SURFACE: &str = r#"{"ICE-B-K26": {"type": "Flat", "vol": 0.3}}"#;
    /// Grid with a vol spike at the middle strike — butterfly-violating.
    const BAD_SURFACE: &str = r#"{"ICE-B-K26": {"type": "Grid", "tenors": [0.25], "moneyness": [-0.2, -0.1, 0.0, 0.1, 0.2], "vols": [[0.30, 0.30, 0.60, 0.30, 0.30]]}}"#;

    #[test]
    fn i0_vol_free_pnl_two_sided() {
        // Side A: a fully-cleared book marks settle-only — deleting
        // vol_surfaces leaves the pnl report byte-identical.
        let dir = tempfile::tempdir().unwrap();
        write_marking_book(dir.path(), "cleared", Some(FLAT_SURFACE), "");
        let with_vols = crate::reports::pnl(&load(&dir.path().join("pricing.toml")).unwrap());
        write_marking_book(dir.path(), "cleared", None, "");
        let without_vols = crate::reports::pnl(&load(&dir.path().join("pricing.toml")).unwrap());
        assert_eq!(with_vols.unwrap(), without_vols.unwrap());

        // Side B: an uncleared position makes the surface a marking
        // requirement — deleting it must fail loudly, not thin the report.
        let dir = tempfile::tempdir().unwrap();
        write_marking_book(dir.path(), "uncleared", Some(FLAT_SURFACE), "");
        let portfolio = load(&dir.path().join("pricing.toml")).unwrap();
        assert!(portfolio.integrity_errors.is_empty());
        let ok = crate::reports::pnl(&portfolio).unwrap();
        assert!(ok.contains("model"), "{ok}");

        write_marking_book(dir.path(), "uncleared", None, "");
        let portfolio = load(&dir.path().join("pricing.toml")).unwrap();
        assert!(
            portfolio
                .integrity_errors
                .iter()
                .any(|e| e.contains("no vol surface")),
            "{:?}",
            portfolio.integrity_errors
        );
        assert!(crate::reports::pnl(&portfolio).is_err());
    }

    // ── vol-free book data plane (architecture §9, task 73) ──────────

    /// A vols history for the marking book: surfaces only, eval day.
    fn write_vols_file(dir: &Path) {
        fs::write(
            dir.join("vols.json"),
            format!(
                r#"{{ "days": [{{
  "valuation_date": "2026-03-07",
  "as_of": "2026-03-07T14:00:00Z",
  "market_prices": {{}},
  "discount_curves": {{}},
  "vol_surfaces": {FLAT_SURFACE}
}}] }}"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn book_warns_on_surface_no_position_needs() {
        // Fully-cleared book with an embedded surface: P&L is settle-only
        // (I0), so the surface is risk-tier data in the wrong file.
        let dir = tempfile::tempdir().unwrap();
        write_marking_book(dir.path(), "cleared", Some(FLAT_SURFACE), "");
        let portfolio = load(&dir.path().join("pricing.toml")).unwrap();
        assert!(
            portfolio
                .warnings
                .iter()
                .any(|w| w.contains("no position needs")),
            "{:?}",
            portfolio.warnings
        );

        // An uncleared model-marked position licenses the surface in-file:
        // no warning.
        let dir = tempfile::tempdir().unwrap();
        write_marking_book(dir.path(), "uncleared", Some(FLAT_SURFACE), "");
        let portfolio = load(&dir.path().join("pricing.toml")).unwrap();
        assert!(
            !portfolio
                .warnings
                .iter()
                .any(|w| w.contains("no position needs")),
            "{:?}",
            portfolio.warnings
        );
    }

    /// The risk.toml companion to the marking book: same data files, plus
    /// vol_data enrichment.
    fn write_risk_config(dir: &Path, extra: &str) {
        fs::write(
            dir.join("risk.toml"),
            format!(
                r#"
instruments = ["instruments.json"]
indices = ["indices.json"]
deals = ["deals.json"]
market = "market.json"
vol_data = ["vols.json"]
{extra}
"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn vol_data_loads_for_risk_tier_only() {
        let dir = tempfile::tempdir().unwrap();
        write_marking_book(dir.path(), "cleared", None, "");
        write_vols_file(dir.path());
        write_risk_config(dir.path(), "");

        // Book config: vol_data is a schema error, not an ignored key —
        // "book data is vol-free" is enforced structurally (§9).
        write_marking_book(dir.path(), "cleared", None, "vol_data = [\"vols.json\"]");
        let err = load(&dir.path().join("pricing.toml")).unwrap_err();
        assert!(
            err.to_string().contains("unknown field `vol_data`"),
            "unhelpful error: {err}"
        );
        write_marking_book(dir.path(), "cleared", None, "");
        let portfolio = load(&dir.path().join("pricing.toml")).unwrap();
        assert!(
            !portfolio
                .market_data
                .as_ref()
                .unwrap()
                .has_vol_surface("ICE-B-K26")
        );

        // Risk tier: overlay merged into the eval day, and no
        // unneeded-surface warning — risk surfaces are beyond marking
        // needs by definition.
        let portfolio = load_risk(&dir.path().join("risk.toml"), None, None).unwrap();
        assert!(
            portfolio
                .market_data
                .as_ref()
                .unwrap()
                .has_vol_surface("ICE-B-K26")
        );
        assert!(
            !portfolio
                .warnings
                .iter()
                .any(|w| w.contains("no position needs")),
            "{:?}",
            portfolio.warnings
        );
    }

    #[test]
    fn legacy_single_day_market_rejected() {
        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());
        // The pre-§9 single-day schema: hard migration, targeted error.
        fs::write(dir.path().join("market.json"), future_day("2026-03-07")).unwrap();
        let err = load(&dir.path().join("pricing.toml")).unwrap_err();
        assert!(
            err.to_string().contains("legacy single-day file"),
            "unhelpful error: {err}"
        );
    }

    #[test]
    fn as_of_selects_history_day() {
        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());
        // Per-day settles so day selection is observable.
        fs::write(
            dir.path().join("market.json"),
            history_json(FIXTURE_DAYS, |date| {
                let settle = format!("72.{}", &date[8..10]);
                future_day(date).replace("72.45", &settle)
            }),
        )
        .unwrap();

        // Default: the last day.
        let portfolio = load_book(&dir.path().join("pricing.toml"), None).unwrap();
        let md = portfolio.market_data.as_ref().unwrap();
        assert_eq!(md.valuation_date().to_string(), "2026-03-07");
        assert_eq!(md.settlement_price("ICE-B-K26").unwrap(), 72.07);

        // --as-of: any contained day.
        let day: Date = "2026-03-04".parse().unwrap();
        let portfolio = load_book(&dir.path().join("pricing.toml"), Some(day)).unwrap();
        let md = portfolio.market_data.as_ref().unwrap();
        assert_eq!(md.valuation_date(), day);
        assert_eq!(md.settlement_price("ICE-B-K26").unwrap(), 72.04);

        // A day outside the history errors with the covered span.
        let missing: Date = "2026-04-01".parse().unwrap();
        let err = load_book(&dir.path().join("pricing.toml"), Some(missing)).unwrap_err();
        assert!(
            err.to_string().contains("2026-03-02..2026-03-07"),
            "unhelpful error: {err}"
        );
    }

    #[test]
    fn vol_data_without_market_errors_for_risk() {
        let dir = tempfile::tempdir().unwrap();
        write_marking_book(dir.path(), "cleared", None, "");
        write_vols_file(dir.path());
        fs::write(
            dir.path().join("risk.toml"),
            r#"
instruments = ["instruments.json"]
indices = ["indices.json"]
deals = ["deals.json"]
vol_data = ["vols.json"]
"#,
        )
        .unwrap();
        let err = load_risk(&dir.path().join("risk.toml"), None, None).unwrap_err();
        assert!(
            err.to_string().contains("without a market file"),
            "unhelpful error: {err}"
        );
    }

    #[test]
    fn no_arb_gate_blocks_bad_surface_for_uncleared_marks() {
        let dir = tempfile::tempdir().unwrap();
        write_marking_book(dir.path(), "uncleared", Some(BAD_SURFACE), "");
        let portfolio = load(&dir.path().join("pricing.toml")).unwrap();
        assert!(
            portfolio
                .integrity_errors
                .iter()
                .any(|e| e.contains("butterfly")),
            "{:?}",
            portfolio.integrity_errors
        );

        // The same surface under a fully-cleared book is risk-side only —
        // no gate, no integrity errors.
        let dir = tempfile::tempdir().unwrap();
        write_marking_book(dir.path(), "cleared", Some(BAD_SURFACE), "");
        let portfolio = load(&dir.path().join("pricing.toml")).unwrap();
        assert!(
            portfolio.integrity_errors.is_empty(),
            "{:?}",
            portfolio.integrity_errors
        );

        // A waiver covering the valuation date downgrades the finding.
        let dir = tempfile::tempdir().unwrap();
        write_marking_book(
            dir.path(),
            "uncleared",
            Some(BAD_SURFACE),
            "[[waivers]]\nfrom = \"2026-03-01\"\nthrough = \"2026-03-31\"\nreason = \"known bad surface\"",
        );
        let portfolio = load(&dir.path().join("pricing.toml")).unwrap();
        assert!(portfolio.integrity_errors.is_empty());
        assert!(
            portfolio
                .warnings
                .iter()
                .any(|w| w.starts_with("waived:") && w.contains("butterfly")),
            "{:?}",
            portfolio.warnings
        );
    }

    #[test]
    fn proxy_mark_uses_twin_settle() {
        let dir = tempfile::tempdir().unwrap();
        // Uncleared option proxied to the cleared future's settle: no
        // surface needed, pnl marks it at 72.45 with source "proxy".
        write_marking_book(
            dir.path(),
            "uncleared",
            None,
            "[proxy_marks]\n\"OPT-BRN-75\" = \"ICE-B-K26\"",
        );
        let portfolio = load(&dir.path().join("pricing.toml")).unwrap();
        assert!(
            portfolio.integrity_errors.is_empty(),
            "{:?}",
            portfolio.integrity_errors
        );
        let out = crate::reports::pnl(&portfolio).unwrap();
        assert!(out.contains("proxy"), "{out}");
        assert!(!out.contains("UNPRICED"), "{out}");
    }

    #[test]
    fn proxy_marks_validation_errors() {
        let check = |extra: &str, needle: &str| {
            let dir = tempfile::tempdir().unwrap();
            write_marking_book(dir.path(), "uncleared", Some(FLAT_SURFACE), extra);
            let err = load(&dir.path().join("pricing.toml"))
                .unwrap_err()
                .to_string();
            assert!(err.contains(needle), "{err}");
        };
        check(
            "[proxy_marks]\n\"NOPE\" = \"ICE-B-K26\"",
            "unknown instrument 'NOPE'",
        );
        check(
            "[proxy_marks]\n\"ICE-B-K26\" = \"ICE-B-K26\"",
            "not uncleared",
        );
        check(
            "[proxy_marks]\n\"OPT-BRN-75\" = \"NOPE\"",
            "proxies unknown instrument 'NOPE'",
        );
        check(
            "[proxy_marks]\n\"OPT-BRN-75\" = \"OPT-BRN-75\"",
            "is not cleared",
        );
    }
}

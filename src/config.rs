use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;

use crate::core;
use crate::instruments::FinancialInstrument;
use crate::market_data::MarketData;
use crate::trades::Deal;

/// TOML configuration pointing to JSON data files.
///
/// Paths are resolved relative to the directory containing the config file.
///
/// ```toml
/// instruments = ["instruments/futures.json", "instruments/bonds.json"]
/// deals = ["deals/march_book.json"]
/// market_data = ["market/2026-03-07.json"]
/// ```
#[derive(Debug, Deserialize)]
pub struct PricingConfig {
    pub instruments: Vec<PathBuf>,
    pub deals: Vec<PathBuf>,
    pub market_data: Vec<PathBuf>,
    #[serde(default)]
    pub reports: Vec<String>,
}

/// Assembled data ready for pricing, returned by [`load`].
#[derive(Debug)]
pub struct Portfolio {
    pub instruments: HashMap<String, Arc<dyn FinancialInstrument>>,
    pub deals: Vec<Deal>,
    pub market_data: MarketData,
    pub warnings: Vec<String>,
    pub reports: Vec<String>,
}

/// Load a TOML config file and assemble a [`Portfolio`].
///
/// Each instrument/deal file may contain a single JSON object or an array.
/// After loading, consistency checks run and warnings are collected.
pub fn load(config_path: &Path) -> core::Result<Portfolio> {
    let config_dir = config_path.parent().unwrap_or_else(|| Path::new("."));

    let toml_str = std::fs::read_to_string(config_path).map_err(|e| {
        core::Error::Config(format!("cannot read {}: {}", config_path.display(), e))
    })?;
    let config: PricingConfig = toml::from_str(&toml_str).map_err(|e| {
        core::Error::Config(format!("invalid config {}: {}", config_path.display(), e))
    })?;

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

    // Load market data (merge multiple files)
    let mut market_data: Option<MarketData> = None;
    for rel_path in &config.market_data {
        let path = config_dir.join(rel_path);
        let md_json = read_json_file(&path)?;
        let md: MarketData = serde_json::from_str(&md_json).map_err(|e| {
            core::Error::Config(format!("invalid market data {}: {}", path.display(), e))
        })?;
        match &mut market_data {
            None => market_data = Some(md),
            Some(existing) => existing.merge(md).map_err(|e| {
                core::Error::Config(format!("market data {}: {}", path.display(), e))
            })?,
        }
    }
    let market_data = market_data
        .ok_or_else(|| core::Error::Config("no market_data files specified".to_string()))?;
    market_data
        .validate()
        .map_err(|e| core::Error::Config(format!("market data: {e}")))?;

    // Consistency checks
    let mut warnings: Vec<String> = Vec::new();

    // Currencies are embedded by value in each instrument's JSON; two
    // instruments declaring the same currency id with different
    // conventions is a silent-mismatch hazard.
    let mut currencies: HashMap<String, (String, crate::reference_data::Currency)> = HashMap::new();
    let mut sorted_ids: Vec<&String> = instruments.keys().collect();
    sorted_ids.sort(); // deterministic "first seen" for stable warnings
    for id in sorted_ids {
        let ccy = instruments[id].currency();
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
    }

    for deal in &deals {
        if !instruments.contains_key(&deal.instrument_id) {
            return Err(core::Error::Config(format!(
                "deal '{}' references unknown instrument '{}'",
                deal.id, deal.instrument_id,
            )));
        }
    }

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

        // Options price via the model, not a quoted market price: check
        // their actual inputs (underlying instrument + vol surface) instead.
        if let Some(opt) = inst
            .as_any()
            .downcast_ref::<crate::instruments::EuropeanOption>()
        {
            if !instruments.contains_key(&opt.underlying) {
                warnings.push(format!(
                    "option '{}': unknown underlying '{}'",
                    id, opt.underlying,
                ));
            }
            if !expired && !market_data.has_vol_surface(&opt.underlying) {
                warnings.push(format!(
                    "option '{}': no vol surface for underlying '{}'",
                    id, opt.underlying,
                ));
            }
            continue;
        }

        if expired {
            if market_data.settlement_price(id).is_err() {
                warnings.push(format!(
                    "instrument '{}': expired but no settlement price",
                    id,
                ));
            }
        } else if market_data.market_price(id).is_err() {
            warnings.push(format!("instrument '{}': no market price", id,));
        }
    }

    Ok(Portfolio {
        instruments,
        deals,
        market_data,
        warnings,
        reports: config.reports,
    })
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

    fn write_test_files(dir: &Path) {
        // Config
        fs::write(
            dir.join("pricing.toml"),
            r#"
instruments = ["instruments.json"]
deals = ["deals.json"]
market_data = ["market.json"]
"#,
        )
        .unwrap();

        // Single instrument
        fs::write(
            dir.join("instruments.json"),
            r#"{
  "type": "Future",
  "id": "ICE-BRN-K26",
  "underlying": "Brent",
  "currency": {"id": "USD", "settlement": "Null", "day_count": "Act360"},
  "settlement": {"venue": "ICE", "session": "SETTLE", "time": "19:30", "timezone": "Europe/London", "payment_lag": "Null"},
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
  "instrument_id": "ICE-BRN-K26",
  "direction": "Buy",
  "quantity": "5",
  "price": "71.80",
  "timestamp": "2026-03-02T10:14:32Z",
  "counterparty": "ICE_CLEAR",
  "venue": "ICE"
}"#,
        )
        .unwrap();

        // Market data
        fs::write(
            dir.join("market.json"),
            r#"{
  "valuation_date": "2026-03-07",
  "as_of": "2026-03-07T14:00:00Z",
  "market_prices": {"ICE-BRN-K26": 72.45},
  "discount_curves": {
    "USD": {
      "base_date": "2026-03-07",
      "day_count": "Act360",
      "pillars": [["2026-06-07", 0.043]]
    }
  }
}"#,
        )
        .unwrap();
    }

    #[test]
    fn load_single_object_files() {
        let dir = tempfile::tempdir().unwrap();
        write_test_files(dir.path());

        let portfolio = load(&dir.path().join("pricing.toml")).unwrap();
        assert_eq!(portfolio.instruments.len(), 1);
        assert!(portfolio.instruments.contains_key("ICE-BRN-K26"));
        assert_eq!(portfolio.deals.len(), 1);
        assert_eq!(portfolio.deals[0].instrument_id, "ICE-BRN-K26");
        assert!(portfolio.warnings.is_empty());
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
    "id": "ICE-BRN-K26",
    "underlying": "Brent",
    "currency": {"id": "USD", "settlement": "Null", "day_count": "Act360"},
    "settlement": {"venue": "ICE", "session": "SETTLE", "time": "19:30", "timezone": "Europe/London", "payment_lag": "Null"},
    "expiry": "2026-03-31",
    "contract_size": "1000",
    "tick_size": "0.01"
  },
  {
    "type": "Future",
    "id": "ICE-BRN-M26",
    "underlying": "Brent",
    "currency": {"id": "USD", "settlement": "Null", "day_count": "Act360"},
    "settlement": {"venue": "ICE", "session": "SETTLE", "time": "19:30", "timezone": "Europe/London", "payment_lag": "Null"},
    "expiry": "2026-05-29",
    "contract_size": "1000",
    "tick_size": "0.01"
  }
]"#,
        )
        .unwrap();

        let portfolio = load(&dir.path().join("pricing.toml")).unwrap();
        assert_eq!(portfolio.instruments.len(), 2);
        assert!(portfolio.instruments.contains_key("ICE-BRN-K26"));
        assert!(portfolio.instruments.contains_key("ICE-BRN-M26"));
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
deals = ["deals.json"]
market_data = ["market.json"]
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

        // Market data with no market_prices or curves
        fs::write(
            dir.path().join("market.json"),
            r#"{
  "valuation_date": "2026-03-07",
  "as_of": "2026-03-07T14:00:00Z",
  "market_prices": {},
  "discount_curves": {}
}"#,
        )
        .unwrap();

        let portfolio = load(&dir.path().join("pricing.toml")).unwrap();
        assert_eq!(portfolio.warnings.len(), 2); // no market price + no curve
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
  {"id": "D1", "instrument_id": "ICE-BRN-K26", "direction": "Buy", "quantity": "1", "price": "70", "timestamp": "2026-03-02T10:00:00Z", "counterparty": "X", "venue": "Y"},
  {"id": "D1", "instrument_id": "ICE-BRN-K26", "direction": "Sell", "quantity": "1", "price": "72", "timestamp": "2026-03-03T10:00:00Z", "counterparty": "X", "venue": "Y"}
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
    "id": "ICE-BRN-K26",
    "underlying": "Brent",
    "currency": {"id": "USD", "settlement": "Null", "day_count": "Act360"},
    "settlement": {"venue": "ICE", "session": "SETTLE", "time": "19:30", "timezone": "Europe/London", "payment_lag": "Null"},
    "expiry": "2026-03-31",
    "contract_size": "1000",
    "tick_size": "0.01"
  },
  {
    "type": "Future",
    "id": "ICE-BRN-M26",
    "underlying": "Brent",
    "currency": {"id": "USD", "settlement": "Null", "day_count": "Act365Fixed"},
    "settlement": {"venue": "ICE", "session": "SETTLE", "time": "19:30", "timezone": "Europe/London", "payment_lag": "Null"},
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
    "id": "ICE-BRN-K26",
    "underlying": "Brent",
    "currency": {"id": "USD", "settlement": "Null", "day_count": "Act360"},
    "settlement": {"venue": "ICE", "session": "SETTLE", "time": "19:30", "timezone": "Europe/London", "payment_lag": "Null"},
    "expiry": "2026-03-31",
    "contract_size": "1000",
    "tick_size": "0.01"
  },
  {
    "type": "EuropeanOption",
    "id": "OPT-NO-VOL",
    "underlying": "ICE-BRN-K26",
    "credit_id": "ICE",
    "currency": {"id": "USD", "settlement": "Null", "day_count": "Act360"},
    "settlement": {"venue": "ICE", "session": "SETTLE", "time": "19:30", "timezone": "Europe/London", "payment_lag": "Null"},
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
    "expiry": "2026-03-27",
    "strike": "75",
    "put_or_call": "Call",
    "exercise_style": "European",
    "option_settlement": "Cash"
  }
]"#,
        )
        .unwrap();

        let portfolio = load(&dir.path().join("pricing.toml")).unwrap();
        assert!(
            portfolio
                .warnings
                .iter()
                .any(|w| w.contains("OPT-NO-VOL") && w.contains("no vol surface")),
            "expected vol surface warning, got: {:?}",
            portfolio.warnings,
        );
        assert!(
            portfolio
                .warnings
                .iter()
                .any(|w| w.contains("OPT-NO-UNDERLYING") && w.contains("unknown underlying")),
            "expected unknown underlying warning, got: {:?}",
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
}

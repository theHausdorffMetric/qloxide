use serde::{Deserialize, Serialize};

use crate::dates::daycount::DayCount;
use crate::dates::rules::DateRule;

/// Currency as reference data — a unit of account, not a tradeable instrument.
///
/// Instruments are denominated in a currency. FX is a currency *pair* instrument.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Currency {
    /// ISO 4217 code (e.g., "USD", "EUR", "GBP").
    pub id: String,
    /// Settlement date rule for this currency.
    pub settlement: DateRule,
    /// Day count convention for interest calculations in this currency.
    pub day_count: DayCount,
}

impl Currency {
    pub fn new(id: &str, settlement: DateRule, day_count: DayCount) -> Currency {
        Currency {
            id: id.to_string(),
            settlement,
            day_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn currency_construction() {
        let usd = Currency::new("USD", DateRule::Null, DayCount::Act360);
        assert_eq!(usd.id, "USD");
    }

    #[test]
    fn currency_serde_roundtrip() {
        let usd = Currency::new("USD", DateRule::Null, DayCount::Act360);
        let json = serde_json::to_string(&usd).unwrap();
        let usd2: Currency = serde_json::from_str(&json).unwrap();
        assert_eq!(usd.id, usd2.id);
        assert_eq!(usd.day_count, usd2.day_count);
    }
}

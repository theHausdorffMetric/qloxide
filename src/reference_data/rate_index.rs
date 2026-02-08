use serde::{Deserialize, Serialize};

use crate::dates::daycount::{Compounding, DayCount};

/// A floating rate index definition.
///
/// Defines the conventions for a floating rate: which currency, what tenor,
/// how it accrues, and where fixings come from.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RateIndex {
    /// Unique identifier (e.g., "USD-SOFR-3M", "EUR-EURIBOR-6M").
    pub id: String,
    /// Currency ISO code.
    pub currency_id: String,
    /// Tenor in months (e.g., 3 for 3M, 6 for 6M, 12 for 1Y).
    pub tenor_months: u32,
    /// Day count convention for accrual.
    pub day_count: DayCount,
    /// Compounding convention.
    pub compounding: Compounding,
    /// Fixing source identifier (e.g., "BLOOMBERG", "REUTERS").
    pub fixing_source: String,
}

impl RateIndex {
    pub fn new(
        id: &str,
        currency_id: &str,
        tenor_months: u32,
        day_count: DayCount,
        compounding: Compounding,
        fixing_source: &str,
    ) -> RateIndex {
        RateIndex {
            id: id.to_string(),
            currency_id: currency_id.to_string(),
            tenor_months,
            day_count,
            compounding,
            fixing_source: fixing_source.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_index_serde_roundtrip() {
        let idx = RateIndex::new(
            "USD-SOFR-3M",
            "USD",
            3,
            DayCount::Act360,
            Compounding::Continuous,
            "BLOOMBERG",
        );
        let json = serde_json::to_string(&idx).unwrap();
        let idx2: RateIndex = serde_json::from_str(&json).unwrap();
        assert_eq!(idx.id, idx2.id);
        assert_eq!(idx.tenor_months, idx2.tenor_months);
    }
}

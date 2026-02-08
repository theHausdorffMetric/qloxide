use serde::{Deserialize, Serialize};

/// Counterparty or issuer identity for credit curve lookups.
///
/// This is reference data, not a tradeable instrument. It identifies whose
/// credit risk is relevant for discounting or CVA calculations.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreditEntity {
    /// Unique identifier (e.g., "JPM", "SHELL").
    pub id: String,
    /// Currency of denomination (ISO code).
    pub currency_id: String,
}

impl CreditEntity {
    pub fn new(id: &str, currency_id: &str) -> CreditEntity {
        CreditEntity {
            id: id.to_string(),
            currency_id: currency_id.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credit_entity_serde_roundtrip() {
        let ce = CreditEntity::new("JPM", "USD");
        let json = serde_json::to_string(&ce).unwrap();
        let ce2: CreditEntity = serde_json::from_str(&json).unwrap();
        assert_eq!(ce.id, ce2.id);
        assert_eq!(ce.currency_id, ce2.currency_id);
    }
}

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::dates::Date;
use crate::instruments::{
    ExerciseStyle, FinancialInstrument, OptionSettlement, PutOrCall, Settlement,
};
use crate::reference_data::Currency;

/// A vanilla option (European or American).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EuropeanOption {
    pub id: String,
    pub underlying: String,
    pub credit_id: String,
    pub currency: Arc<Currency>,
    pub settlement: Settlement,
    pub expiry: Date,
    pub strike: f64,
    pub put_or_call: PutOrCall,
    pub exercise_style: ExerciseStyle,
    pub option_settlement: OptionSettlement,
    /// Payment date (may differ from expiry for cash-settled options).
    pub pay_date: Date,
}

impl EuropeanOption {
    pub fn new(
        id: &str,
        underlying: &str,
        credit_id: &str,
        currency: Arc<Currency>,
        settlement: Settlement,
        expiry: Date,
        strike: f64,
        put_or_call: PutOrCall,
        option_settlement: OptionSettlement,
        pay_date: Date,
    ) -> EuropeanOption {
        EuropeanOption {
            id: id.to_string(),
            underlying: underlying.to_string(),
            credit_id: credit_id.to_string(),
            currency,
            settlement,
            expiry,
            strike,
            put_or_call,
            exercise_style: ExerciseStyle::European,
            option_settlement,
            pay_date,
        }
    }

    /// Intrinsic value at a given spot price.
    pub fn intrinsic(&self, spot: f64) -> f64 {
        match self.put_or_call {
            PutOrCall::Call => (spot - self.strike).max(0.0),
            PutOrCall::Put => (self.strike - spot).max(0.0),
        }
    }
}

#[typetag::serde]
impl FinancialInstrument for EuropeanOption {
    fn id(&self) -> &str {
        &self.id
    }

    fn currency(&self) -> &Currency {
        &self.currency
    }

    fn settlement(&self) -> &Settlement {
        &self.settlement
    }

    fn maturity(&self) -> Option<Date> {
        Some(self.expiry)
    }

    fn instrument_type(&self) -> &str {
        "EuropeanOption"
    }
}

use std::sync::Arc;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::dates::Date;
use crate::instruments::{
    Clearing, ExerciseStyle, FinancialInstrument, OptionSettlement, PutOrCall, Settlement,
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
    /// Where the contract clears — required, no default: every instrument
    /// states its nature explicitly (it decides marking + completeness checks).
    pub clearing: Clearing,
    pub expiry: Date,
    pub strike: Decimal,
    pub put_or_call: PutOrCall,
    pub exercise_style: ExerciseStyle,
    pub option_settlement: OptionSettlement,
}

impl EuropeanOption {
    #[allow(clippy::too_many_arguments)] // all fields are pub; use a struct literal if preferred
    pub fn new(
        id: &str,
        underlying: &str,
        credit_id: &str,
        currency: Arc<Currency>,
        settlement: Settlement,
        clearing: Clearing,
        expiry: Date,
        strike: Decimal,
        put_or_call: PutOrCall,
        option_settlement: OptionSettlement,
    ) -> EuropeanOption {
        EuropeanOption {
            id: id.to_string(),
            underlying: underlying.to_string(),
            credit_id: credit_id.to_string(),
            currency,
            settlement,
            clearing,
            expiry,
            strike,
            put_or_call,
            exercise_style: ExerciseStyle::European,
            option_settlement,
        }
    }

    /// Payment date: expiry adjusted by the settlement payment lag.
    /// Derived rather than stored, so it cannot disagree with the
    /// settlement conventions in hand-edited JSON.
    pub fn pay_date(&self) -> Date {
        self.settlement.pay_date(self.expiry)
    }

    /// Intrinsic value at a given underlying price.
    pub fn intrinsic(&self, underlying: Decimal) -> Decimal {
        let zero = Decimal::ZERO;
        match self.put_or_call {
            PutOrCall::Call => (underlying - self.strike).max(zero),
            PutOrCall::Put => (self.strike - underlying).max(zero),
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

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

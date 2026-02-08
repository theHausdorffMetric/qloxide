use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::dates::Date;
use crate::instruments::{FinancialInstrument, Settlement};
use crate::reference_data::Currency;

/// An FX forward: exchange of two currency amounts on a future date.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FxForward {
    pub id: String,
    pub credit_id: String,
    /// Base currency (the one we "buy").
    pub base_currency: Arc<Currency>,
    /// Quote currency (the one we "sell").
    pub quote_currency: Arc<Currency>,
    pub settlement: Settlement,
    /// Amount of base currency.
    pub base_amount: f64,
    /// Amount of quote currency (negative = we pay).
    pub quote_amount: f64,
    /// Delivery date.
    pub delivery_date: Date,
}

impl FxForward {
    pub fn new(
        id: &str,
        credit_id: &str,
        base_currency: Arc<Currency>,
        quote_currency: Arc<Currency>,
        settlement: Settlement,
        base_amount: f64,
        quote_amount: f64,
        delivery_date: Date,
    ) -> FxForward {
        FxForward {
            id: id.to_string(),
            credit_id: credit_id.to_string(),
            base_currency,
            quote_currency,
            settlement,
            base_amount,
            quote_amount,
            delivery_date,
        }
    }

    /// Implied forward rate: quote_amount / base_amount.
    pub fn forward_rate(&self) -> f64 {
        self.quote_amount / self.base_amount
    }
}

#[typetag::serde]
impl FinancialInstrument for FxForward {
    fn id(&self) -> &str {
        &self.id
    }

    fn currency(&self) -> &Currency {
        // Convention: the FX forward is denominated in the quote currency
        &self.quote_currency
    }

    fn settlement(&self) -> &Settlement {
        &self.settlement
    }

    fn maturity(&self) -> Option<Date> {
        Some(self.delivery_date)
    }

    fn instrument_type(&self) -> &str {
        "FxForward"
    }
}

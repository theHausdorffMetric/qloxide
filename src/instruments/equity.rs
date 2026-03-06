use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::dates::Date;
use crate::instruments::{FinancialInstrument, Settlement};
use crate::reference_data::Currency;

/// An equity or commodity underlier.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Equity {
    pub id: String,
    pub credit_id: String,
    pub currency: Arc<Currency>,
    pub settlement: Settlement,
}

impl Equity {
    pub fn new(
        id: &str,
        credit_id: &str,
        currency: Arc<Currency>,
        settlement: Settlement,
    ) -> Equity {
        Equity {
            id: id.to_string(),
            credit_id: credit_id.to_string(),
            currency,
            settlement,
        }
    }
}

#[typetag::serde]
impl FinancialInstrument for Equity {
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
        None // equities don't mature
    }

    fn instrument_type(&self) -> &str {
        "Equity"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

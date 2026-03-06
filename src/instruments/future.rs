use std::sync::Arc;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::dates::Date;
use crate::instruments::{FinancialInstrument, Settlement};
use crate::reference_data::Currency;

/// An exchange-traded future contract.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Future {
    pub id: String,
    pub underlying: String,
    pub currency: Arc<Currency>,
    pub settlement: Settlement,
    pub expiry: Date,
    pub contract_size: Decimal,
    pub tick_size: Decimal,
}

impl Future {
    pub fn new(
        id: &str,
        underlying: &str,
        currency: Arc<Currency>,
        settlement: Settlement,
        expiry: Date,
        contract_size: Decimal,
        tick_size: Decimal,
    ) -> Future {
        Future {
            id: id.to_string(),
            underlying: underlying.to_string(),
            currency,
            settlement,
            expiry,
            contract_size,
            tick_size,
        }
    }
}

#[typetag::serde]
impl FinancialInstrument for Future {
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
        "Future"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

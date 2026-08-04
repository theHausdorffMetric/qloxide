use std::sync::Arc;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::dates::Date;
use crate::instruments::{ClearingStatus, FinancialInstrument, Settlement};
use crate::reference_data::{Currency, IndexRef};

/// An exchange-traded future contract.
///
/// Structurally identical across products: what the contract settles
/// against lives in the settlement-index registry behind `underlying`;
/// the contract itself owns exactly one date (`expiry`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Future {
    pub id: String,
    /// What the contract settles against: a settlement-index id, resolved
    /// against the book's registry at load (dangling = config error).
    pub underlying: IndexRef,
    pub currency: Arc<Currency>,
    pub settlement: Settlement,
    /// Where the contract clears — required, no default: every instrument
    /// states its nature explicitly (it decides marking + completeness checks).
    pub clearing: ClearingStatus,
    pub expiry: Date,
    pub contract_size: Decimal,
    pub tick_size: Decimal,
}

impl Future {
    #[allow(clippy::too_many_arguments)] // all fields are pub; use a struct literal if preferred
    pub fn new(
        id: &str,
        underlying: &str,
        currency: Arc<Currency>,
        settlement: Settlement,
        clearing: ClearingStatus,
        expiry: Date,
        contract_size: Decimal,
        tick_size: Decimal,
    ) -> Future {
        Future {
            id: id.to_string(),
            underlying: IndexRef::new(underlying),
            currency,
            settlement,
            clearing,
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

    fn clearing(&self) -> Option<ClearingStatus> {
        Some(self.clearing)
    }

    fn instrument_type(&self) -> &str {
        "Future"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

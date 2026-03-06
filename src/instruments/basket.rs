use std::sync::Arc;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::dates::Date;
use crate::instruments::{FinancialInstrument, Settlement};
use crate::reference_data::Currency;

/// A weighted basket of instruments.
///
/// Recursive: a basket can contain other baskets.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Basket {
    pub id: String,
    pub credit_id: String,
    pub currency: Arc<Currency>,
    pub settlement: Settlement,
    /// Components: (weight, instrument).
    pub components: Vec<(Decimal, Arc<dyn FinancialInstrument>)>,
}

impl Basket {
    pub fn new(
        id: &str,
        credit_id: &str,
        currency: Arc<Currency>,
        settlement: Settlement,
        components: Vec<(Decimal, Arc<dyn FinancialInstrument>)>,
    ) -> Basket {
        Basket {
            id: id.to_string(),
            credit_id: credit_id.to_string(),
            currency,
            settlement,
            components,
        }
    }

    /// Number of components.
    pub fn len(&self) -> usize {
        self.components.len()
    }

    /// True if the basket has no components.
    pub fn is_empty(&self) -> bool {
        self.components.is_empty()
    }
}

#[typetag::serde]
impl FinancialInstrument for Basket {
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
        // Basket maturity = latest component maturity
        self.components
            .iter()
            .filter_map(|(_, inst)| inst.maturity())
            .max()
    }

    fn instrument_type(&self) -> &str {
        "Basket"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

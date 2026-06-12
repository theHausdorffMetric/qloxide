use std::sync::Arc;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::cashflows::Frequency;
use crate::dates::Date;
use crate::dates::daycount::DayCount;
use crate::instruments::{FinancialInstrument, Settlement};
use crate::reference_data::Currency;

/// A fixed-rate bond.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bond {
    pub id: String,
    pub credit_id: String,
    pub currency: Arc<Currency>,
    pub settlement: Settlement,
    /// Issue date.
    pub issue_date: Date,
    /// Maturity date.
    pub maturity_date: Date,
    /// Face/notional value.
    pub face_value: Decimal,
    /// Annual coupon rate (e.g., 0.05 for 5%).
    pub coupon_rate: Decimal,
    /// Day count convention for coupon accrual.
    pub day_count: DayCount,
    /// Coupon payment frequency.
    pub frequency: Frequency,
}

impl Bond {
    pub fn new(
        id: &str,
        credit_id: &str,
        currency: Arc<Currency>,
        settlement: Settlement,
        issue_date: Date,
        maturity_date: Date,
        face_value: Decimal,
        coupon_rate: Decimal,
        day_count: DayCount,
        frequency: Frequency,
    ) -> Bond {
        Bond {
            id: id.to_string(),
            credit_id: credit_id.to_string(),
            currency,
            settlement,
            issue_date,
            maturity_date,
            face_value,
            coupon_rate,
            day_count,
            frequency,
        }
    }

    /// Coupon amount per period.
    pub fn coupon_amount(&self) -> Decimal {
        self.face_value * self.coupon_rate / Decimal::from(self.frequency.per_year())
    }
}

#[typetag::serde]
impl FinancialInstrument for Bond {
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
        Some(self.maturity_date)
    }

    fn instrument_type(&self) -> &str {
        "Bond"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

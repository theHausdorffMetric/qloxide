use std::sync::Arc;

use serde::{Deserialize, Serialize};

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
    pub face_value: f64,
    /// Annual coupon rate (e.g., 0.05 for 5%).
    pub coupon_rate: f64,
    /// Day count convention for coupon accrual.
    pub day_count: DayCount,
    /// Number of coupon payments per year.
    pub frequency: u32,
}

impl Bond {
    pub fn new(
        id: &str,
        credit_id: &str,
        currency: Arc<Currency>,
        settlement: Settlement,
        issue_date: Date,
        maturity_date: Date,
        face_value: f64,
        coupon_rate: f64,
        day_count: DayCount,
        frequency: u32,
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
    pub fn coupon_amount(&self) -> f64 {
        self.face_value * self.coupon_rate / self.frequency as f64
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
}

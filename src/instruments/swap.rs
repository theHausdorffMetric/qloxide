use std::sync::Arc;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::dates::Date;
use crate::dates::daycount::DayCount;
use crate::cashflows::Frequency;
use crate::instruments::{FinancialInstrument, Settlement};
use crate::reference_data::Currency;

/// Direction of a swap leg from the perspective of the holder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PayReceive {
    Pay,
    Receive,
}

/// A fixed leg of an interest rate swap.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FixedLeg {
    /// Notional principal.
    pub notional: Decimal,
    /// Fixed rate (e.g., 0.03 for 3%).
    pub rate: Decimal,
    /// Day count convention.
    pub day_count: DayCount,
    /// Payment frequency.
    pub frequency: Frequency,
    /// Start date of accrual.
    pub start_date: Date,
    /// End date of accrual.
    pub end_date: Date,
    /// Pay or receive.
    pub direction: PayReceive,
}

/// A floating leg of an interest rate swap.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FloatingLeg {
    /// Notional principal.
    pub notional: Decimal,
    /// Rate index id (e.g., "USD-SOFR-3M").
    pub rate_index_id: String,
    /// Spread over the floating rate (e.g., 0.001 for 10bp).
    pub spread: Decimal,
    /// Day count convention.
    pub day_count: DayCount,
    /// Payment frequency.
    pub frequency: Frequency,
    /// Start date of accrual.
    pub start_date: Date,
    /// End date of accrual.
    pub end_date: Date,
    /// Pay or receive.
    pub direction: PayReceive,
}

/// An interest rate swap: exchange of fixed vs floating cash flows.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Swap {
    pub id: String,
    pub credit_id: String,
    pub currency: Arc<Currency>,
    pub settlement: Settlement,
    pub fixed_leg: FixedLeg,
    pub floating_leg: FloatingLeg,
}

impl Swap {
    pub fn new(
        id: &str,
        credit_id: &str,
        currency: Arc<Currency>,
        settlement: Settlement,
        fixed_leg: FixedLeg,
        floating_leg: FloatingLeg,
    ) -> Swap {
        Swap {
            id: id.to_string(),
            credit_id: credit_id.to_string(),
            currency,
            settlement,
            fixed_leg,
            floating_leg,
        }
    }

    /// Effective date (earliest leg start).
    pub fn effective_date(&self) -> Date {
        if self.fixed_leg.start_date < self.floating_leg.start_date {
            self.fixed_leg.start_date
        } else {
            self.floating_leg.start_date
        }
    }

    /// Termination date (latest leg end).
    pub fn termination_date(&self) -> Date {
        if self.fixed_leg.end_date > self.floating_leg.end_date {
            self.fixed_leg.end_date
        } else {
            self.floating_leg.end_date
        }
    }
}

#[typetag::serde]
impl FinancialInstrument for Swap {
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
        Some(self.termination_date())
    }

    fn instrument_type(&self) -> &str {
        "Swap"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

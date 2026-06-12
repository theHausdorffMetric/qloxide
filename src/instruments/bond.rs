use std::sync::Arc;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::cashflows::{CashFlow, CashFlowSchedule, Frequency};
use crate::dates::Date;
use crate::dates::daycount::DayCount;
use crate::dates::rules::DateRule;
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
    #[allow(clippy::too_many_arguments)] // all fields are pub; use a struct literal if preferred
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

    /// Contractual cash flows: periodic coupons plus principal at maturity.
    ///
    /// Coupon dates are generated backward from maturity via
    /// [`CashFlowSchedule::generate`] (short first stub for odd tenors),
    /// unadjusted. Each coupon accrues
    /// `face_value * coupon_rate * year_fraction(period)` under the bond's
    /// day count convention, computed exactly in `Decimal` via
    /// [`DayCount::accrue`].
    pub fn cash_flows(&self) -> Vec<CashFlow> {
        let schedule = CashFlowSchedule::generate(
            self.issue_date,
            self.maturity_date,
            self.frequency,
            &DateRule::Null,
        );
        let periods = schedule.periods();
        let base = self.face_value * self.coupon_rate;
        let mut flows = Vec::with_capacity(periods.len() + 1);
        for (start, end) in periods {
            flows.push(CashFlow::new(
                self.day_count.accrue(base, start, end),
                self.currency.clone(),
                end,
            ));
        }
        flows.push(CashFlow::new(
            self.face_value,
            self.currency.clone(),
            self.maturity_date,
        ));
        flows
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dates::rules::DateRule;
    use std::sync::Arc;

    fn test_bond() -> Bond {
        let usd = Arc::new(Currency::new(
            "USD",
            DateRule::Null,
            DayCount::Act360,
        ));
        Bond::new(
            "UST-5Y",
            "US-GOVT",
            usd,
            Settlement::otc(),
            Date::new(2025, 1, 1),
            Date::new(2030, 1, 1),
            Decimal::from(100),
            "0.05".parse().unwrap(),
            DayCount::Thirty360,
            Frequency::SemiAnnual,
        )
    }

    #[test]
    fn cash_flows_coupons_plus_principal() {
        let bond = test_bond();
        let flows = bond.cash_flows();

        // 5 years semi-annual = 10 coupons + 1 principal
        assert_eq!(flows.len(), 11);

        // 30/360 semi-annual periods are exactly 0.5: coupon = 100 * 0.05 * 0.5
        let coupon: Decimal = "2.5".parse().unwrap();
        for flow in &flows[..10] {
            assert_eq!(flow.amount, coupon);
            assert_eq!(flow.currency.id, "USD");
        }

        // First coupon and final flows
        assert_eq!(flows[0].pay_date, Date::new(2025, 7, 1));
        assert_eq!(flows[9].pay_date, Date::new(2030, 1, 1));
        assert_eq!(flows[10].amount, Decimal::from(100));
        assert_eq!(flows[10].pay_date, Date::new(2030, 1, 1));
    }

    #[test]
    fn cash_flows_short_first_stub() {
        // 14 months semi-annual: backward generation gives a short FIRST
        // period (2 months), then two full 6-month periods.
        let usd = Arc::new(Currency::new("USD", DateRule::Null, DayCount::Act360));
        let bond = Bond::new(
            "STUB", "X", usd, Settlement::otc(),
            Date::new(2025, 1, 1),
            Date::new(2026, 3, 1),
            Decimal::from(100),
            "0.06".parse().unwrap(),
            DayCount::Thirty360,
            Frequency::SemiAnnual,
        );
        let flows = bond.cash_flows();
        // 3 coupons + principal
        assert_eq!(flows.len(), 4);
        assert_eq!(flows[0].pay_date, Date::new(2025, 3, 1)); // short stub end
        assert_eq!(flows[1].pay_date, Date::new(2025, 9, 1));
        assert_eq!(flows[2].pay_date, Date::new(2026, 3, 1));
        // Stub coupon accrues 2/12 of a year under 30/360: 100*0.06*60/360 = 1
        assert_eq!(flows[0].amount, Decimal::from(1));
        // Full coupons: 100*0.06*0.5 = 3
        assert_eq!(flows[1].amount, Decimal::from(3));
    }
}

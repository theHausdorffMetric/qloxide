use std::sync::Arc;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::dates::Date;
use crate::dates::rules::DateRule;
use crate::reference_data::Currency;

/// The atomic unit of finance: a cash amount in a currency on a date.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CashFlow {
    /// Cash amount (positive = receive, negative = pay).
    pub amount: Decimal,
    /// Currency of the cash flow.
    pub currency: Arc<Currency>,
    /// Payment date.
    pub pay_date: Date,
}

impl CashFlow {
    pub fn new(amount: Decimal, currency: Arc<Currency>, pay_date: Date) -> CashFlow {
        CashFlow {
            amount,
            currency,
            pay_date,
        }
    }
}

/// Frequency for generating periodic dates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Frequency {
    Monthly,
    Quarterly,
    SemiAnnual,
    Annual,
}

impl Frequency {
    /// Number of months in one period.
    pub fn months(&self) -> i32 {
        match self {
            Frequency::Monthly => 1,
            Frequency::Quarterly => 3,
            Frequency::SemiAnnual => 6,
            Frequency::Annual => 12,
        }
    }
}

/// A schedule of dates for cash flow generation (e.g., swap coupon dates).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CashFlowSchedule {
    /// Generated adjusted dates.
    pub dates: Vec<Date>,
}

impl CashFlowSchedule {
    /// Generate a schedule of dates from `start` to `end` at the given frequency,
    /// adjusted according to the date rule.
    ///
    /// Dates are generated backwards from `end` to ensure the final period
    /// always ends on `end`. The start date is included as the first element.
    pub fn generate(
        start: Date,
        end: Date,
        frequency: Frequency,
        rule: &DateRule,
    ) -> CashFlowSchedule {
        let months = frequency.months();
        let mut unadjusted = vec![end];

        // Generate backwards from end
        let mut current = add_months(end, -months);
        while current > start {
            unadjusted.push(current);
            current = add_months(current, -months);
        }
        unadjusted.push(start);
        unadjusted.reverse();

        // Adjust all dates except start and end using the rule
        let mut dates: Vec<Date> = Vec::with_capacity(unadjusted.len());
        for (i, &d) in unadjusted.iter().enumerate() {
            if i == 0 || i == unadjusted.len() - 1 {
                dates.push(d);
            } else {
                dates.push(rule.apply(d));
            }
        }

        CashFlowSchedule { dates }
    }

    /// Generate a simple schedule: just start and end.
    pub fn bullet(start: Date, end: Date) -> CashFlowSchedule {
        CashFlowSchedule {
            dates: vec![start, end],
        }
    }

    /// The accrual periods: pairs of (period_start, period_end).
    pub fn periods(&self) -> Vec<(Date, Date)> {
        self.dates.windows(2).map(|w| (w[0], w[1])).collect()
    }
}

/// Add months to a date, clamping to the last day of the target month.
fn add_months(date: Date, months: i32) -> Date {
    let total_months = date.year() as i32 * 12 + (date.month() as i32 - 1) + months;
    let target_year = (total_months / 12) as i16;
    let target_month = (total_months % 12 + 1) as i8;

    // Clamp day to the target month's length
    let day = date.day();
    let max_day = days_in_month(target_year, target_month);
    let clamped_day = day.min(max_day);

    Date::new(target_year, target_month, clamped_day)
}

fn days_in_month(year: i16, month: i8) -> i8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => unreachable!("invalid month: {}", month),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dates::calendar::Calendar;
    use crate::dates::daycount::DayCount;

    fn usd() -> Arc<Currency> {
        Arc::new(Currency::new("USD", DateRule::Null, DayCount::Act360))
    }

    #[test]
    fn cashflow_construction() {
        let cf = CashFlow::new(Decimal::from(1_000_000), usd(), Date::new(2025, 12, 15));
        assert_eq!(cf.amount, Decimal::from(1_000_000));
        assert_eq!(cf.currency.id, "USD");
        assert_eq!(cf.pay_date, Date::new(2025, 12, 15));
    }

    #[test]
    fn cashflow_serde_roundtrip() {
        let cf = CashFlow::new(Decimal::from(50_000), usd(), Date::new(2025, 6, 15));
        let json = serde_json::to_string(&cf).unwrap();
        let cf2: CashFlow = serde_json::from_str(&json).unwrap();
        assert_eq!(cf.amount, cf2.amount);
        assert_eq!(cf.pay_date, cf2.pay_date);
    }

    #[test]
    fn schedule_quarterly() {
        let start = Date::new(2025, 1, 15);
        let end = Date::new(2026, 1, 15);
        let sched = CashFlowSchedule::generate(
            start,
            end,
            Frequency::Quarterly,
            &DateRule::Null,
        );
        // 1Y with quarterly = 5 dates (start + 4 periods)
        assert_eq!(sched.dates.len(), 5);
        assert_eq!(sched.dates[0], start);
        assert_eq!(sched.dates[4], end);
        assert_eq!(sched.dates[1], Date::new(2025, 4, 15));
        assert_eq!(sched.dates[2], Date::new(2025, 7, 15));
        assert_eq!(sched.dates[3], Date::new(2025, 10, 15));
    }

    #[test]
    fn schedule_semiannual() {
        let start = Date::new(2025, 6, 1);
        let end = Date::new(2027, 6, 1);
        let sched = CashFlowSchedule::generate(
            start,
            end,
            Frequency::SemiAnnual,
            &DateRule::Null,
        );
        assert_eq!(sched.dates.len(), 5); // start + 4 semi-annual dates
        assert_eq!(sched.periods().len(), 4);
    }

    #[test]
    fn schedule_bullet() {
        let sched = CashFlowSchedule::bullet(
            Date::new(2025, 1, 1),
            Date::new(2030, 1, 1),
        );
        assert_eq!(sched.dates.len(), 2);
        assert_eq!(sched.periods().len(), 1);
    }

    #[test]
    fn add_months_end_of_month() {
        // Jan 31 + 1 month = Feb 28 (non-leap) or Feb 29 (leap)
        let d = add_months(Date::new(2025, 1, 31), 1);
        assert_eq!(d, Date::new(2025, 2, 28));

        let d_leap = add_months(Date::new(2024, 1, 31), 1);
        assert_eq!(d_leap, Date::new(2024, 2, 29));
    }

    #[test]
    fn schedule_with_business_day_adjustment() {
        let start = Date::new(2025, 1, 15);
        let end = Date::new(2025, 7, 15);
        let rule = DateRule::ModifiedFollowing {
            calendar: Calendar::Weekday,
        };
        let sched = CashFlowSchedule::generate(
            start,
            end,
            Frequency::Quarterly,
            &rule,
        );
        // All intermediate dates should be weekdays
        for &d in &sched.dates[1..sched.dates.len() - 1] {
            let wd = d.weekday();
            assert!(wd >= 1 && wd <= 5, "date {} is not a weekday", d);
        }
    }
}

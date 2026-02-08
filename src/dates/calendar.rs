use serde::{Deserialize, Serialize};

use crate::dates::Date;

/// Holiday calendar for business day calculations.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Calendar {
    /// Every day is a business day (no holidays, no weekends).
    EveryDay,

    /// Weekdays only (Monday–Friday). No specific holidays.
    Weekday,

    /// Weekdays excluding a specific holiday list.
    WeekdayAndHoliday {
        name: String,
        holidays: Vec<Date>, // must be sorted
    },

    /// Volatility calendar: weights holidays differently for vol time.
    Volatility {
        name: String,
        calendar: Box<Calendar>,
        holiday_weight: f64,
    },
}

impl Calendar {
    /// Calendar name.
    pub fn name(&self) -> &str {
        match self {
            Calendar::EveryDay => "EveryDay",
            Calendar::Weekday => "Weekday",
            Calendar::WeekdayAndHoliday { name, .. } => name,
            Calendar::Volatility { name, .. } => name,
        }
    }

    /// True if the given date is a holiday (non-business day) under this calendar.
    pub fn is_holiday(&self, date: Date) -> bool {
        match self {
            Calendar::EveryDay => false,
            Calendar::Weekday => {
                let wd = date.weekday();
                wd == 6 || wd == 7 // Saturday or Sunday
            }
            Calendar::WeekdayAndHoliday { holidays, .. } => {
                let wd = date.weekday();
                if wd == 6 || wd == 7 {
                    return true;
                }
                holidays.binary_search(&date).is_ok()
            }
            Calendar::Volatility { calendar, .. } => calendar.is_holiday(date),
        }
    }

    /// Step forward or backward by `step` business days from `from`.
    /// If `slip_forward` is true, when `from` is a holiday it slips to the
    /// next business day; otherwise it slips to the previous.
    pub fn step(&self, from: Date, step: i32, slip_forward: bool) -> Date {
        let direction = if slip_forward { 1 } else { -1 };

        // First slip to a business day
        let mut current = from;
        while self.is_holiday(current) {
            current = current + direction;
        }

        // Then step by business days
        if step >= 0 {
            for _ in 0..step {
                current = current + 1;
                while self.is_holiday(current) {
                    current = current + 1;
                }
            }
        } else {
            for _ in 0..(-step) {
                current = current - 1;
                while self.is_holiday(current) {
                    current = current - 1;
                }
            }
        }

        current
    }

    /// Count business days between `from` and `to` (exclusive of `to`).
    /// Negative if `to < from`.
    pub fn count_business_days(&self, from: Date, to: Date) -> i32 {
        if from == to {
            return 0;
        }

        let (start, end, sign) = if from < to {
            (from, to, 1)
        } else {
            (to, from, -1)
        };

        match self {
            Calendar::EveryDay => sign * (end - start),
            Calendar::Weekday => sign * count_weekdays(start, end),
            Calendar::WeekdayAndHoliday { holidays, .. } => {
                let weekdays = count_weekdays(start, end);
                // Subtract holidays that fall on weekdays in the range
                let holiday_count = holidays
                    .iter()
                    .filter(|&&h| {
                        h >= start && h < end && {
                            let wd = h.weekday();
                            (1..=5).contains(&wd)
                        }
                    })
                    .count() as i32;
                sign * (weekdays - holiday_count)
            }
            Calendar::Volatility { calendar, .. } => {
                calendar.count_business_days(from, to) * sign.signum()
            }
        }
    }

    /// Weight of a single day for volatility time calculations.
    pub fn day_weight(&self, date: Date) -> f64 {
        match self {
            Calendar::EveryDay => 1.0,
            Calendar::Weekday => {
                if self.is_holiday(date) { 0.0 } else { 1.0 }
            }
            Calendar::WeekdayAndHoliday { .. } => {
                if self.is_holiday(date) { 0.0 } else { 1.0 }
            }
            Calendar::Volatility {
                calendar,
                holiday_weight,
                ..
            } => {
                if calendar.is_holiday(date) {
                    *holiday_weight
                } else {
                    1.0
                }
            }
        }
    }

    /// Standard basis (business days per year) for this calendar type.
    pub fn standard_basis(&self) -> f64 {
        match self {
            Calendar::EveryDay => 365.0,
            Calendar::Weekday => 252.0,
            Calendar::WeekdayAndHoliday { .. } => 252.0,
            Calendar::Volatility { .. } => 252.0,
        }
    }

    /// Year fraction between two date-day-fraction points.
    pub fn year_fraction(&self, from: Date, from_frac: f64, to: Date, to_frac: f64) -> f64 {
        let business_days =
            self.count_business_days_fractional(from, from_frac, to, to_frac);
        business_days / self.standard_basis()
    }

    /// Count business days between fractional date points.
    fn count_business_days_fractional(
        &self,
        from: Date,
        from_frac: f64,
        to: Date,
        to_frac: f64,
    ) -> f64 {
        if from == to {
            return (to_frac - from_frac) * self.day_weight(from);
        }

        // Partial first day
        let first_partial = (1.0 - from_frac) * self.day_weight(from);

        // Full intermediate days
        let mut full_days = 0.0;
        let mut d = from + 1;
        while d < to {
            full_days += self.day_weight(d);
            d = d + 1;
        }

        // Partial last day
        let last_partial = to_frac * self.day_weight(to);

        first_partial + full_days + last_partial
    }
}

/// Count weekdays (Mon–Fri) from `from` (inclusive) to `to` (exclusive).
fn count_weekdays(from: Date, to: Date) -> i32 {
    let total_days = to - from;
    if total_days <= 0 {
        return 0;
    }

    let full_weeks = total_days / 7;
    let remaining = total_days % 7;

    let start_wd = from.weekday(); // 1=Mon .. 7=Sun

    // Count weekend days in the remaining partial week
    let mut weekend_in_remainder = 0;
    for i in 0..remaining {
        let wd = (start_wd - 1 + i as i8) % 7 + 1;
        if wd == 6 || wd == 7 {
            weekend_in_remainder += 1;
        }
    }

    full_weeks * 5 + (remaining - weekend_in_remainder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everyday_no_holidays() {
        let cal = Calendar::EveryDay;
        let saturday = Date::new(2025, 6, 14);
        assert!(!cal.is_holiday(saturday));
    }

    #[test]
    fn weekday_weekend_is_holiday() {
        let cal = Calendar::Weekday;
        let saturday = Date::new(2025, 6, 14);
        let sunday = Date::new(2025, 6, 15);
        let monday = Date::new(2025, 6, 16);
        assert!(cal.is_holiday(saturday));
        assert!(cal.is_holiday(sunday));
        assert!(!cal.is_holiday(monday));
    }

    #[test]
    fn weekday_and_holiday() {
        let cal = Calendar::WeekdayAndHoliday {
            name: "NYSE".to_string(),
            holidays: vec![Date::new(2025, 12, 25)], // Christmas
        };
        assert!(cal.is_holiday(Date::new(2025, 12, 25))); // Thursday holiday
        assert!(!cal.is_holiday(Date::new(2025, 12, 24))); // Wednesday, not holiday
        assert!(cal.is_holiday(Date::new(2025, 12, 27))); // Saturday
    }

    #[test]
    fn step_forward() {
        let cal = Calendar::Weekday;
        // Friday + 1 business day = Monday
        let friday = Date::new(2025, 6, 13);
        assert_eq!(cal.step(friday, 1, true), Date::new(2025, 6, 16));
    }

    #[test]
    fn step_from_weekend_slips_forward() {
        let cal = Calendar::Weekday;
        let saturday = Date::new(2025, 6, 14);
        // Slip forward to Monday, then step 0
        assert_eq!(cal.step(saturday, 0, true), Date::new(2025, 6, 16));
    }

    #[test]
    fn step_from_weekend_slips_backward() {
        let cal = Calendar::Weekday;
        let saturday = Date::new(2025, 6, 14);
        // Slip backward to Friday, then step 0
        assert_eq!(cal.step(saturday, 0, false), Date::new(2025, 6, 13));
    }

    #[test]
    fn step_backward() {
        let cal = Calendar::Weekday;
        // Monday - 1 business day = Friday
        let monday = Date::new(2025, 6, 16);
        assert_eq!(cal.step(monday, -1, true), Date::new(2025, 6, 13));
    }

    #[test]
    fn count_business_days_one_week() {
        let cal = Calendar::Weekday;
        let monday = Date::new(2025, 6, 16);
        let next_monday = Date::new(2025, 6, 23);
        assert_eq!(cal.count_business_days(monday, next_monday), 5);
    }

    #[test]
    fn count_business_days_with_holidays() {
        let cal = Calendar::WeekdayAndHoliday {
            name: "Test".to_string(),
            holidays: vec![Date::new(2025, 6, 18)], // Wednesday
        };
        let monday = Date::new(2025, 6, 16);
        let next_monday = Date::new(2025, 6, 23);
        assert_eq!(cal.count_business_days(monday, next_monday), 4);
    }

    #[test]
    fn count_business_days_negative() {
        let cal = Calendar::Weekday;
        let monday = Date::new(2025, 6, 16);
        let next_monday = Date::new(2025, 6, 23);
        assert_eq!(cal.count_business_days(next_monday, monday), -5);
    }

    #[test]
    fn volatility_calendar() {
        let base = Calendar::Weekday;
        let vol_cal = Calendar::Volatility {
            name: "VolCal".to_string(),
            calendar: Box::new(base),
            holiday_weight: 0.1,
        };
        let saturday = Date::new(2025, 6, 14);
        let monday = Date::new(2025, 6, 16);
        assert_eq!(vol_cal.day_weight(saturday), 0.1);
        assert_eq!(vol_cal.day_weight(monday), 1.0);
    }

    #[test]
    fn calendar_serde_roundtrip() {
        let cal = Calendar::WeekdayAndHoliday {
            name: "LSE".to_string(),
            holidays: vec![Date::new(2025, 12, 25), Date::new(2025, 12, 26)],
        };
        let json = serde_json::to_string(&cal).unwrap();
        let cal2: Calendar = serde_json::from_str(&json).unwrap();
        assert_eq!(cal.name(), cal2.name());
        assert!(cal2.is_holiday(Date::new(2025, 12, 25)));
    }
}

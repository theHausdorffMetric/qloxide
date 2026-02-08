use serde::{Deserialize, Serialize};

use crate::dates::Date;
use crate::dates::calendar::Calendar;

/// Rule for adjusting dates to business days.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum DateRule {
    /// No adjustment — use the date as-is.
    Null,

    /// Step by a number of business days, optionally slipping forward or back.
    BusinessDays {
        calendar: Calendar,
        step: i32,
        slip_forward: bool,
    },

    /// Modified Following: move to the next business day, but if that crosses
    /// a month boundary, move to the previous business day instead.
    ModifiedFollowing { calendar: Calendar },
}

impl DateRule {
    /// Apply this rule to adjust a date.
    pub fn apply(&self, date: Date) -> Date {
        match self {
            DateRule::Null => date,
            DateRule::BusinessDays {
                calendar,
                step,
                slip_forward,
            } => calendar.step(date, *step, *slip_forward),
            DateRule::ModifiedFollowing { calendar } => {
                let adjusted = calendar.step(date, 0, true);
                if adjusted.month() != date.month() {
                    calendar.step(date, 0, false)
                } else {
                    adjusted
                }
            }
        }
    }

    /// Convenience: next business day (step=0, slip forward).
    pub fn next(calendar: Calendar) -> DateRule {
        DateRule::BusinessDays {
            calendar,
            step: 0,
            slip_forward: true,
        }
    }

    /// Convenience: previous business day (step=0, slip backward).
    pub fn prev(calendar: Calendar) -> DateRule {
        DateRule::BusinessDays {
            calendar,
            step: 0,
            slip_forward: false,
        }
    }

    /// Convenience: step forward N business days.
    pub fn step_forward(calendar: Calendar, step: u32) -> DateRule {
        DateRule::BusinessDays {
            calendar,
            step: step as i32,
            slip_forward: true,
        }
    }

    /// Convenience: step backward N business days.
    pub fn step_back(calendar: Calendar, step: u32) -> DateRule {
        DateRule::BusinessDays {
            calendar,
            step: -(step as i32),
            slip_forward: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_rule() {
        let rule = DateRule::Null;
        let d = Date::new(2025, 6, 14); // Saturday
        assert_eq!(rule.apply(d), d);
    }

    #[test]
    fn next_business_day() {
        let rule = DateRule::next(Calendar::Weekday);
        let saturday = Date::new(2025, 6, 14);
        assert_eq!(rule.apply(saturday), Date::new(2025, 6, 16)); // Monday
    }

    #[test]
    fn prev_business_day() {
        let rule = DateRule::prev(Calendar::Weekday);
        let saturday = Date::new(2025, 6, 14);
        assert_eq!(rule.apply(saturday), Date::new(2025, 6, 13)); // Friday
    }

    #[test]
    fn step_forward_two_days() {
        let rule = DateRule::step_forward(Calendar::Weekday, 2);
        let friday = Date::new(2025, 6, 13);
        assert_eq!(rule.apply(friday), Date::new(2025, 6, 17)); // Tuesday
    }

    #[test]
    fn modified_following_no_month_cross() {
        let rule = DateRule::ModifiedFollowing {
            calendar: Calendar::Weekday,
        };
        let saturday = Date::new(2025, 6, 14);
        assert_eq!(rule.apply(saturday), Date::new(2025, 6, 16)); // Monday
    }

    #[test]
    fn modified_following_month_boundary() {
        let rule = DateRule::ModifiedFollowing {
            calendar: Calendar::Weekday,
        };
        // Aug 31, 2025 is Sunday. Next bday is Sep 1 (crosses month).
        // So it should go to prev bday: Aug 29 (Friday).
        let end_of_month = Date::new(2025, 8, 31);
        assert_eq!(rule.apply(end_of_month), Date::new(2025, 8, 29));
    }

    #[test]
    fn daterule_serde_roundtrip() {
        let rule = DateRule::step_forward(Calendar::Weekday, 2);
        let json = serde_json::to_string(&rule).unwrap();
        let rule2: DateRule = serde_json::from_str(&json).unwrap();
        let d = Date::new(2025, 6, 13);
        assert_eq!(rule.apply(d), rule2.apply(d));
    }
}

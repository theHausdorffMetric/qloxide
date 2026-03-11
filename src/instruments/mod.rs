pub mod bond;
pub mod equity;
pub mod future;
pub mod fx;
pub mod option;
pub mod swap;
pub mod basket;

use serde::{Deserialize, Serialize};

use crate::core;
use crate::dates::{Date, Zoned};
use crate::dates::calendar::Calendar;
use crate::dates::rules::DateRule;
use crate::reference_data::Currency;

/// Settlement conventions for a financial instrument.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settlement {
    /// Venue or exchange (e.g., "ICE", "CME", "PLATTS").
    pub venue: String,
    /// Settlement session name (e.g., "SETTLE", "SINGAPORE_CLOSE").
    pub session: String,
    /// Time of day for settlement (HH:MM format).
    pub time: String,
    /// IANA timezone (e.g., "Europe/London").
    pub timezone: String,
    /// Payment lag: how many business days after trade/expiry until cash moves.
    pub payment_lag: DateRule,
}

impl Settlement {
    pub fn new(venue: &str, session: &str, time: &str, timezone: &str,
               payment_lag: DateRule) -> Settlement {
        Settlement {
            venue: venue.to_string(),
            session: session.to_string(),
            time: time.to_string(),
            timezone: timezone.to_string(),
            payment_lag,
        }
    }

    /// Combine this settlement's time and timezone with a calendar date to
    /// produce a `Zoned` datetime representing the settlement instant.
    pub fn at_date(&self, date: Date) -> core::Result<Zoned> {
        let parts: Vec<&str> = self.time.split(':').collect();
        if parts.len() != 2 {
            return Err(core::Error::Instrument(
                format!("invalid settlement time '{}': expected HH:MM", self.time),
            ));
        }
        let hour: i8 = parts[0].parse().map_err(|_| {
            core::Error::Instrument(format!("invalid hour in '{}'", self.time))
        })?;
        let minute: i8 = parts[1].parse().map_err(|_| {
            core::Error::Instrument(format!("invalid minute in '{}'", self.time))
        })?;
        let tz = jiff::tz::TimeZone::get(&self.timezone).map_err(|e| {
            core::Error::Instrument(format!("invalid timezone '{}': {}", self.timezone, e))
        })?;
        let dt = date.inner().at(hour, minute, 0, 0);
        let zoned = dt.to_zoned(tz).map_err(|e| {
            core::Error::Instrument(format!(
                "cannot resolve {} {} in {}: {}",
                date, self.time, self.timezone, e
            ))
        })?;
        Ok(Zoned::from_jiff(zoned))
    }

    /// Compute the payment date by applying the settlement lag.
    pub fn pay_date(&self, date: Date) -> Date {
        self.payment_lag.apply(date)
    }

    /// Default settlement for OTC instruments (T+2 weekday calendar).
    pub fn otc() -> Settlement {
        Settlement {
            venue: "OTC".to_string(),
            session: "CLOSE".to_string(),
            time: "17:00".to_string(),
            timezone: "America/New_York".to_string(),
            payment_lag: DateRule::step_forward(Calendar::Weekday, 2),
        }
    }
}

/// Put or call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PutOrCall {
    Put,
    Call,
}

/// Exercise style for options.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExerciseStyle {
    European,
    American,
}

/// How an option settles at expiry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OptionSettlement {
    Cash,
    Physical,
}

/// Core trait for all financial instruments.
///
/// Uses `typetag` for automatic tagged JSON serialization of trait objects.
/// New instrument types can be added by implementing this trait with
/// `#[typetag::serde]` — no changes to existing code required.
#[typetag::serde(tag = "type")]
pub trait FinancialInstrument: Send + Sync + std::fmt::Debug {
    /// Unique instrument identifier.
    fn id(&self) -> &str;

    /// Currency the instrument is denominated in.
    fn currency(&self) -> &Currency;

    /// Settlement conventions.
    fn settlement(&self) -> &Settlement;

    /// Maturity or expiry date, if applicable.
    fn maturity(&self) -> Option<Date>;

    /// Human-readable instrument type name.
    fn instrument_type(&self) -> &str;

    /// Downcast support.
    fn as_any(&self) -> &dyn std::any::Any;
}

// Re-export instrument types for convenience
pub use basket::Basket;
pub use bond::Bond;
pub use equity::Equity;
pub use future::Future;
pub use fx::FxForward;
pub use option::EuropeanOption;
pub use swap::{FixedLeg, FloatingLeg, PayReceive, Swap};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dates::Timestamp;

    #[test]
    fn settlement_at_date_ice_london() {
        let s = Settlement::new("ICE", "SETTLE", "19:30", "Europe/London", DateRule::Null);
        let z = s.at_date(Date::new(2026, 3, 31)).unwrap();
        // 2026-03-31 is BST (UTC+1), so 19:30 London = 18:30 UTC
        let expected = Timestamp::parse("2026-03-31T18:30:00Z").unwrap();
        assert_eq!(z.timestamp(), expected);
    }

    #[test]
    fn settlement_at_date_otc_new_york() {
        let s = Settlement::otc(); // 17:00 America/New_York, T+2
        // 2026-03-07 is EST (UTC-5), so 17:00 NY = 22:00 UTC
        let z = s.at_date(Date::new(2026, 3, 7)).unwrap();
        let expected = Timestamp::parse("2026-03-07T22:00:00Z").unwrap();
        assert_eq!(z.timestamp(), expected);
    }

    #[test]
    fn settlement_at_date_otc_new_york_dst() {
        let s = Settlement::otc(); // 17:00 America/New_York, T+2
        // 2026-07-01 is EDT (UTC-4), so 17:00 NY = 21:00 UTC
        let z = s.at_date(Date::new(2026, 7, 1)).unwrap();
        let expected = Timestamp::parse("2026-07-01T21:00:00Z").unwrap();
        assert_eq!(z.timestamp(), expected);
    }

    #[test]
    fn settlement_at_date_invalid_time() {
        let s = Settlement::new("ICE", "SETTLE", "bad", "Europe/London", DateRule::Null);
        assert!(s.at_date(Date::new(2026, 3, 7)).is_err());
    }

    #[test]
    fn settlement_at_date_invalid_timezone() {
        let s = Settlement::new("ICE", "SETTLE", "19:30", "Fake/Zone", DateRule::Null);
        assert!(s.at_date(Date::new(2026, 3, 7)).is_err());
    }

    #[test]
    fn pay_date_t_plus_2_midweek() {
        // Wednesday 2026-03-11 + T+2 = Friday 2026-03-13
        let s = Settlement::otc();
        assert_eq!(s.pay_date(Date::new(2026, 3, 11)), Date::new(2026, 3, 13));
    }

    #[test]
    fn pay_date_t_plus_2_over_weekend() {
        // Thursday 2026-03-12 + T+2 = Monday 2026-03-16 (skips Sat/Sun)
        let s = Settlement::otc();
        assert_eq!(s.pay_date(Date::new(2026, 3, 12)), Date::new(2026, 3, 16));
    }

    #[test]
    fn pay_date_null_rule() {
        let s = Settlement::new("ICE", "SETTLE", "19:30", "Europe/London", DateRule::Null);
        let d = Date::new(2026, 3, 11);
        assert_eq!(s.pay_date(d), d);
    }
}

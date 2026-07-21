pub mod basket;
pub mod bond;
pub mod equity;
pub mod future;
pub mod fx;
pub mod option;
pub mod swap;

use serde::{Deserialize, Serialize};

use crate::core;
use crate::dates::calendar::Calendar;
use crate::dates::rules::DateRule;
use crate::dates::{Date, Time, Zoned};
use crate::reference_data::Currency;

/// Settlement conventions for a financial instrument.
///
/// Deserialization validates the timezone against the IANA database, so a
/// `Settlement` loaded from JSON is known-resolvable. (Direct struct-literal
/// construction can bypass this; `at_date` re-checks.)
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "SettlementRaw")]
pub struct Settlement {
    /// Venue or exchange (e.g., "ICE", "CME", "PLATTS").
    pub venue: String,
    /// Settlement session name (e.g., "SETTLE", "SINGAPORE_CLOSE").
    pub session: String,
    /// Time of day for settlement.
    pub time: Time,
    /// IANA timezone (e.g., "Europe/London").
    pub timezone: String,
    /// Payment lag: how many business days after trade/expiry until cash moves.
    pub payment_lag: DateRule,
}

/// Serde helper: field-for-field mirror of [`Settlement`]; the conversion
/// validates the timezone name.
#[derive(Deserialize)]
struct SettlementRaw {
    venue: String,
    session: String,
    time: Time,
    timezone: String,
    payment_lag: DateRule,
}

impl TryFrom<SettlementRaw> for Settlement {
    type Error = core::Error;

    fn try_from(raw: SettlementRaw) -> core::Result<Settlement> {
        jiff::tz::TimeZone::get(&raw.timezone).map_err(|e| {
            core::Error::Instrument(format!("invalid timezone '{}': {}", raw.timezone, e))
        })?;
        Ok(Settlement {
            venue: raw.venue,
            session: raw.session,
            time: raw.time,
            timezone: raw.timezone,
            payment_lag: raw.payment_lag,
        })
    }
}

impl Settlement {
    /// Construct from literals. Panics on an invalid time string — use
    /// JSON deserialization for untrusted input (cf. `Date::new` vs
    /// `Date::try_new`).
    pub fn new(
        venue: &str,
        session: &str,
        time: &str,
        timezone: &str,
        payment_lag: DateRule,
    ) -> Settlement {
        Settlement {
            venue: venue.to_string(),
            session: session.to_string(),
            time: time.parse().expect("invalid settlement time"),
            timezone: timezone.to_string(),
            payment_lag,
        }
    }

    /// Combine this settlement's time and timezone with a calendar date to
    /// produce a `Zoned` datetime representing the settlement instant.
    pub fn at_date(&self, date: Date) -> core::Result<Zoned> {
        let tz = jiff::tz::TimeZone::get(&self.timezone).map_err(|e| {
            core::Error::Instrument(format!("invalid timezone '{}': {}", self.timezone, e))
        })?;
        let dt = date.inner().at(self.time.hour(), self.time.minute(), 0, 0);
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
            time: Time::new(17, 0),
            timezone: "America/New_York".to_string(),
            payment_lag: DateRule::step_forward(Calendar::Weekday, 2),
        }
    }
}

/// Whether the contract is centrally cleared: `Cleared` means a CCP's daily
/// settlement publication provides the official mark; `Uncleared` means no
/// official settle exists — such instruments mark to model.
///
/// This is an *intrinsic fact* of the contract — margining, calendars and
/// final-settlement mechanics follow from it. Which CCP is not recorded
/// here: venue identity is owned by [`Settlement`] (the fixing source) and
/// the credit relationship by the deal, while the *mark source* actually
/// used for a position on a given day (settle, model, or a configured
/// proxy) is derived from this fact plus book config — a valuation-time
/// concern, not an instrument attribute.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClearingStatus {
    Cleared,
    Uncleared,
}

impl std::fmt::Display for ClearingStatus {
    /// Matches the JSON vocabulary (`"cleared"` / `"uncleared"`).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ClearingStatus::Cleared => "cleared",
            ClearingStatus::Uncleared => "uncleared",
        })
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

    /// Clearing status, for instrument types that carry the fact
    /// (exchange-tradeable derivatives). `None` for types without a
    /// clearing dimension (equities, bonds, …). Drives the official
    /// marking policy: cleared → settlement price, uncleared → model.
    fn clearing(&self) -> Option<ClearingStatus> {
        None
    }

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
    #[should_panic(expected = "invalid settlement time")]
    fn settlement_new_invalid_time_panics() {
        Settlement::new("ICE", "SETTLE", "bad", "Europe/London", DateRule::Null);
    }

    #[test]
    #[should_panic(expected = "invalid settlement time")]
    fn settlement_new_out_of_range_hour_panics() {
        Settlement::new("ICE", "SETTLE", "25:30", "Europe/London", DateRule::Null);
    }

    #[test]
    #[should_panic(expected = "invalid settlement time")]
    fn settlement_new_out_of_range_minute_panics() {
        Settlement::new("ICE", "SETTLE", "19:75", "Europe/London", DateRule::Null);
    }

    #[test]
    fn settlement_deserialize_rejects_bad_time() {
        let json = r#"{"venue":"ICE","session":"SETTLE","time":"25:30","timezone":"Europe/London","payment_lag":"Null"}"#;
        let result: Result<Settlement, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn settlement_deserialize_rejects_bad_timezone() {
        let json = r#"{"venue":"ICE","session":"SETTLE","time":"19:30","timezone":"Fake/Zone","payment_lag":"Null"}"#;
        let result: Result<Settlement, _> = serde_json::from_str(json);
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid timezone"), "got: {err}");
    }

    #[test]
    fn settlement_serde_roundtrip_keeps_hh_mm() {
        let s = Settlement::new("ICE", "SETTLE", "19:30", "Europe/London", DateRule::Null);
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"19:30\""), "got: {json}");
        let s2: Settlement = serde_json::from_str(&json).unwrap();
        assert_eq!(s.time, s2.time);
    }

    #[test]
    fn settlement_at_date_invalid_timezone() {
        // Struct-literal construction bypasses deserialize validation;
        // at_date must still fail cleanly.
        let s = Settlement {
            venue: "ICE".to_string(),
            session: "SETTLE".to_string(),
            time: crate::dates::Time::new(19, 30),
            timezone: "Fake/Zone".to_string(),
            payment_lag: DateRule::Null,
        };
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

pub mod calendar;
pub mod daycount;
pub mod rules;

use std::fmt;
use std::ops::{Add, Sub};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::core;

/// Calendar date newtype wrapping `jiff::civil::Date`.
///
/// Used for curve pillars, holidays, settlement dates — anywhere a calendar
/// day is needed without time-of-day or timezone context.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date(jiff::civil::Date);

impl Date {
    /// Create a date from year, month, day. Panics on invalid input.
    pub fn new(year: i16, month: i8, day: i8) -> Date {
        Self::try_new(year, month, day).expect("invalid date")
    }

    /// Create a date from year, month, day. Returns error on invalid input.
    pub fn try_new(year: i16, month: i8, day: i8) -> core::Result<Date> {
        jiff::civil::Date::new(year, month, day)
            .map(Date)
            .map_err(|e| core::Error::Date(e.to_string()))
    }

    /// The underlying jiff date.
    pub fn inner(&self) -> jiff::civil::Date {
        self.0
    }

    /// Year component.
    pub fn year(&self) -> i16 {
        self.0.year()
    }

    /// Month component (1–12).
    pub fn month(&self) -> i8 {
        self.0.month()
    }

    /// Day component (1–31).
    pub fn day(&self) -> i8 {
        self.0.day()
    }

    /// Day of week: 1=Monday .. 7=Sunday (ISO 8601).
    pub fn weekday(&self) -> i8 {
        self.0.weekday().to_monday_one_offset()
    }

    /// Year fraction between two dates using a simple Act/365 basis.
    /// For more precise day count conventions, use `DayCount`.
    pub fn act365_year_fraction(&self, other: Date) -> f64 {
        let days = (other.0 - self.0).get_days();
        days as f64 / 365.0
    }

    /// Convert to a UTC midnight Timestamp.
    pub fn as_of_midnight(&self) -> Timestamp {
        let dt = self.0.at(0, 0, 0, 0);
        let zoned = dt.to_zoned(jiff::tz::TimeZone::UTC)
            .expect("midnight UTC is always unambiguous");
        Timestamp(zoned.timestamp())
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Debug for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Date({})", self.0)
    }
}

impl FromStr for Date {
    type Err = core::Error;

    fn from_str(s: &str) -> core::Result<Date> {
        s.parse::<jiff::civil::Date>()
            .map(Date)
            .map_err(|e| core::Error::Date(e.to_string()))
    }
}

// Date + i32 days
impl Add<i32> for Date {
    type Output = Date;

    fn add(self, days: i32) -> Date {
        let span = jiff::Span::new().days(days);
        Date(self.0.checked_add(span).expect("date overflow"))
    }
}

// Date - i32 days
impl Sub<i32> for Date {
    type Output = Date;

    fn sub(self, days: i32) -> Date {
        let span = jiff::Span::new().days(days);
        Date(self.0.checked_sub(span).expect("date underflow"))
    }
}

// Date - Date = i32 days
impl Sub<Date> for Date {
    type Output = i32;

    fn sub(self, other: Date) -> i32 {
        (self.0 - other.0).get_days()
    }
}

// Custom serde: serialize as "YYYY-MM-DD" string
impl Serialize for Date {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for Date {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse::<jiff::civil::Date>()
            .map(Date)
            .map_err(serde::de::Error::custom)
    }
}

/// UTC timestamp newtype wrapping `jiff::Timestamp`.
///
/// Used for trade execution times, event logging — anywhere an absolute
/// instant in time is needed without timezone context.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(jiff::Timestamp);

impl Timestamp {
    /// Create from a jiff Timestamp.
    pub fn from_jiff(ts: jiff::Timestamp) -> Timestamp {
        Timestamp(ts)
    }

    /// Create from seconds since Unix epoch.
    pub fn from_second(second: i64) -> core::Result<Timestamp> {
        jiff::Timestamp::from_second(second)
            .map(Timestamp)
            .map_err(|e| core::Error::Date(e.to_string()))
    }

    /// Parse from an RFC 3339 string.
    pub fn parse(s: &str) -> core::Result<Timestamp> {
        s.parse::<jiff::Timestamp>()
            .map(Timestamp)
            .map_err(|e| core::Error::Date(e.to_string()))
    }

    /// The underlying jiff timestamp.
    pub fn inner(&self) -> jiff::Timestamp {
        self.0
    }

    /// Extract the calendar date (UTC).
    pub fn date(&self) -> Date {
        Date(self.0.to_zoned(jiff::tz::TimeZone::UTC).date())
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Debug for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Timestamp({})", self.0)
    }
}

impl Serialize for Timestamp {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse::<jiff::Timestamp>()
            .map(Timestamp)
            .map_err(serde::de::Error::custom)
    }
}

/// Zoned datetime newtype wrapping `jiff::Zoned`.
///
/// Used for settlement times, observation times — anywhere a datetime
/// tied to a specific timezone is needed.
#[derive(Clone, Debug)]
pub struct Zoned(jiff::Zoned);

impl Zoned {
    /// Create from a jiff Zoned.
    pub fn from_jiff(z: jiff::Zoned) -> Zoned {
        Zoned(z)
    }

    /// Parse from a string like "2025-06-14T19:30:00[Europe/London]".
    pub fn parse(s: &str) -> core::Result<Zoned> {
        s.parse::<jiff::Zoned>()
            .map(Zoned)
            .map_err(|e| core::Error::Date(e.to_string()))
    }

    /// The underlying jiff Zoned.
    pub fn inner(&self) -> &jiff::Zoned {
        &self.0
    }

    /// Extract the calendar date in the zoned timezone.
    pub fn date(&self) -> Date {
        Date(self.0.date())
    }

    /// Convert to a UTC Timestamp.
    pub fn timestamp(&self) -> Timestamp {
        Timestamp(self.0.timestamp())
    }

    /// The timezone.
    pub fn timezone(&self) -> jiff::tz::TimeZone {
        self.0.time_zone().clone()
    }
}

impl fmt::Display for Zoned {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Serialize for Zoned {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for Zoned {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse::<jiff::Zoned>()
            .map(Zoned)
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_new() {
        let d = Date::new(2025, 6, 14);
        assert_eq!(d.year(), 2025);
        assert_eq!(d.month(), 6);
        assert_eq!(d.day(), 14);
    }

    #[test]
    fn date_try_new_invalid() {
        assert!(Date::try_new(2025, 2, 30).is_err());
    }

    #[test]
    fn date_arithmetic() {
        let d = Date::new(2025, 6, 14);
        let d2 = d + 7;
        assert_eq!(d2, Date::new(2025, 6, 21));
        assert_eq!(d2 - d, 7);
        assert_eq!(d2 - 7, d);
    }

    #[test]
    fn date_weekday() {
        // 2025-06-14 is a Saturday = 6
        let d = Date::new(2025, 6, 14);
        assert_eq!(d.weekday(), 6);
        // 2025-06-16 is a Monday = 1
        let d2 = Date::new(2025, 6, 16);
        assert_eq!(d2.weekday(), 1);
    }

    #[test]
    fn date_ordering() {
        let d1 = Date::new(2025, 1, 1);
        let d2 = Date::new(2025, 12, 31);
        assert!(d1 < d2);
    }

    #[test]
    fn date_from_str() {
        let d: Date = "2025-06-14".parse().unwrap();
        assert_eq!(d, Date::new(2025, 6, 14));
    }

    #[test]
    fn date_display() {
        let d = Date::new(2025, 6, 14);
        assert_eq!(d.to_string(), "2025-06-14");
    }

    #[test]
    fn date_serde_roundtrip() {
        let d = Date::new(2025, 6, 14);
        let json = serde_json::to_string(&d).unwrap();
        assert_eq!(json, "\"2025-06-14\"");
        let d2: Date = serde_json::from_str(&json).unwrap();
        assert_eq!(d, d2);
    }

    #[test]
    fn date_as_of_midnight() {
        let d = Date::new(2026, 3, 7);
        let ts = d.as_of_midnight();
        assert_eq!(ts, Timestamp::parse("2026-03-07T00:00:00Z").unwrap());
    }

    #[test]
    fn timestamp_parse() {
        let ts = Timestamp::parse("2025-06-14T14:32:07Z").unwrap();
        assert_eq!(ts.date(), Date::new(2025, 6, 14));
    }

    #[test]
    fn timestamp_serde_roundtrip() {
        let ts = Timestamp::from_second(1718370727).unwrap();
        let json = serde_json::to_string(&ts).unwrap();
        let ts2: Timestamp = serde_json::from_str(&json).unwrap();
        assert_eq!(ts, ts2);
    }

    #[test]
    fn zoned_parse() {
        let z = Zoned::parse("2025-06-14T19:30:00[Europe/London]").unwrap();
        assert_eq!(z.date(), Date::new(2025, 6, 14));
    }

    #[test]
    fn zoned_serde_roundtrip() {
        let z = Zoned::parse("2025-06-14T19:30:00[Europe/London]").unwrap();
        let json = serde_json::to_string(&z).unwrap();
        let z2: Zoned = serde_json::from_str(&json).unwrap();
        assert_eq!(z.timestamp(), z2.timestamp());
    }
}

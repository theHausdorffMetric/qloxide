use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::dates::Date;

/// Day count convention for computing year fractions between dates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DayCount {
    /// Actual/360 — used by money markets, LIBOR, EURIBOR
    Act360,
    /// Actual/365 Fixed — used by GBP LIBOR, many bond markets
    Act365Fixed,
    /// 30/360 (Bond Basis) — used by US corporate bonds
    Thirty360,
    /// Actual/Actual (ISDA) — used by government bonds
    ActActIsda,
    // BUS/252 (BRL) deliberately omitted: it counts *business* days and
    // therefore needs a Calendar parameter. Add as `Bus252 { calendar }`
    // when Brazilian instruments are needed.
}

impl DayCount {
    /// Compute the year fraction between two dates under this convention.
    pub fn year_fraction(&self, from: Date, to: Date) -> f64 {
        match self {
            DayCount::Act360 => {
                let days = to - from;
                days as f64 / 360.0
            }
            DayCount::Act365Fixed => {
                let days = to - from;
                days as f64 / 365.0
            }
            DayCount::Thirty360 => {
                thirty360_day_count(from, to) as f64 / 360.0
            }
            DayCount::ActActIsda => {
                act_act_isda(from, to)
            }
        }
    }

    /// Accrual amount `base * year_fraction(from, to)` computed entirely in
    /// `Decimal`, with the division performed last. Contractual amounts that
    /// are exact in decimal (e.g. a 30/360 half-year coupon) stay exact,
    /// with no f64 round-trip noise. Use this for cash flow generation;
    /// use [`DayCount::year_fraction`] for pricing math.
    pub fn accrue(&self, base: Decimal, from: Date, to: Date) -> Decimal {
        match self {
            DayCount::Act360 => base * Decimal::from(to - from) / Decimal::from(360),
            DayCount::Act365Fixed => base * Decimal::from(to - from) / Decimal::from(365),
            DayCount::Thirty360 => {
                base * Decimal::from(thirty360_day_count(from, to)) / Decimal::from(360)
            }
            DayCount::ActActIsda => act_act_isda_accrue(base, from, to),
        }
    }

    /// The fixed denominator for this convention (where applicable).
    pub fn basis(&self) -> f64 {
        match self {
            DayCount::Act360 => 360.0,
            DayCount::Act365Fixed => 365.0,
            DayCount::Thirty360 => 360.0,
            DayCount::ActActIsda => 365.25, // approximate
        }
    }
}

/// 30/360 day count: assumes 30-day months, 360-day years.
fn thirty360_day_count(from: Date, to: Date) -> i32 {
    let (y1, m1, mut d1) = (from.year() as i32, from.month() as i32, from.day() as i32);
    let (y2, m2, mut d2) = (to.year() as i32, to.month() as i32, to.day() as i32);

    if d1 == 31 {
        d1 = 30;
    }
    if d2 == 31 && d1 >= 30 {
        d2 = 30;
    }

    360 * (y2 - y1) + 30 * (m2 - m1) + (d2 - d1)
}

/// Act/Act ISDA: actual days in each year, weighted by year length.
fn act_act_isda(from: Date, to: Date) -> f64 {
    if from == to {
        return 0.0;
    }

    let y1 = from.year();
    let y2 = to.year();

    if y1 == y2 {
        let days = (to - from) as f64;
        let year_days = year_length(y1) as f64;
        return days / year_days;
    }

    // Fraction of first year
    let end_of_y1 = Date::new(y1 + 1, 1, 1);
    let frac_first = (end_of_y1 - from) as f64 / year_length(y1) as f64;

    // Full intermediate years
    let full_years = (y2 - y1 - 1).max(0) as f64;

    // Fraction of last year
    let start_of_y2 = Date::new(y2, 1, 1);
    let frac_last = (to - start_of_y2) as f64 / year_length(y2) as f64;

    frac_first + full_years + frac_last
}

/// Act/Act ISDA accrual in Decimal: same structure as [`act_act_isda`],
/// each yearly term divided last.
fn act_act_isda_accrue(base: Decimal, from: Date, to: Date) -> Decimal {
    if from == to {
        return Decimal::ZERO;
    }

    let y1 = from.year();
    let y2 = to.year();

    if y1 == y2 {
        return base * Decimal::from(to - from) / Decimal::from(year_length(y1));
    }

    let end_of_y1 = Date::new(y1 + 1, 1, 1);
    let first = base * Decimal::from(end_of_y1 - from) / Decimal::from(year_length(y1));

    let full_years = base * Decimal::from((y2 - y1 - 1).max(0));

    let start_of_y2 = Date::new(y2, 1, 1);
    let last = base * Decimal::from(to - start_of_y2) / Decimal::from(year_length(y2));

    first + full_years + last
}

fn is_leap_year(year: i16) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn year_length(year: i16) -> i32 {
    if is_leap_year(year) { 366 } else { 365 }
}

/// Compounding convention for interest rate calculations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Compounding {
    /// Continuous: df = exp(-r * t)
    Continuous,
    /// Simple: df = 1 / (1 + r * t)
    Simple,
    /// Annual: df = 1 / (1 + r)^t
    Annual,
    /// Semi-annual: df = 1 / (1 + r/2)^(2t)
    SemiAnnual,
    /// Quarterly: df = 1 / (1 + r/4)^(4t)
    Quarterly,
    /// Daily: df = 1 / (1 + r/365)^(365t)
    Daily,
}

impl Compounding {
    /// Convert a rate under this compounding to a discount factor.
    pub fn df(&self, rate: f64, year_fraction: f64) -> f64 {
        match self {
            Compounding::Continuous => (-rate * year_fraction).exp(),
            Compounding::Simple => 1.0 / (1.0 + rate * year_fraction),
            Compounding::Annual => (1.0 + rate).powf(-year_fraction),
            Compounding::SemiAnnual => (1.0 + rate / 2.0).powf(-2.0 * year_fraction),
            Compounding::Quarterly => (1.0 + rate / 4.0).powf(-4.0 * year_fraction),
            Compounding::Daily => (1.0 + rate / 365.0).powf(-365.0 * year_fraction),
        }
    }

    /// Convert a discount factor to a rate under this compounding.
    pub fn rate_from_df(&self, df: f64, year_fraction: f64) -> f64 {
        if year_fraction.abs() < 1e-15 {
            return 0.0;
        }
        match self {
            Compounding::Continuous => -df.ln() / year_fraction,
            Compounding::Simple => (1.0 / df - 1.0) / year_fraction,
            Compounding::Annual => df.powf(-1.0 / year_fraction) - 1.0,
            Compounding::SemiAnnual => 2.0 * (df.powf(-1.0 / (2.0 * year_fraction)) - 1.0),
            Compounding::Quarterly => 4.0 * (df.powf(-1.0 / (4.0 * year_fraction)) - 1.0),
            Compounding::Daily => 365.0 * (df.powf(-1.0 / (365.0 * year_fraction)) - 1.0),
        }
    }

    /// Convert a rate from one compounding convention to another.
    pub fn convert(&self, rate: f64, year_fraction: f64, target: Compounding) -> f64 {
        let df = self.df(rate, year_fraction);
        target.rate_from_df(df, year_fraction)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn act360() {
        let from = Date::new(2025, 1, 1);
        let to = Date::new(2025, 7, 1);
        let yf = DayCount::Act360.year_fraction(from, to);
        assert_eq!(yf, 181.0 / 360.0);
    }

    #[test]
    fn act365() {
        let from = Date::new(2025, 1, 1);
        let to = Date::new(2025, 7, 1);
        let yf = DayCount::Act365Fixed.year_fraction(from, to);
        assert_eq!(yf, 181.0 / 365.0);
    }

    #[test]
    fn thirty360_basic() {
        let from = Date::new(2025, 1, 15);
        let to = Date::new(2025, 7, 15);
        let yf = DayCount::Thirty360.year_fraction(from, to);
        assert!((yf - 0.5).abs() < 1e-10); // 6 months = 180/360
    }

    #[test]
    fn act_act_isda_same_year() {
        let from = Date::new(2025, 1, 1);
        let to = Date::new(2025, 7, 1);
        let yf = DayCount::ActActIsda.year_fraction(from, to);
        assert!((yf - 181.0 / 365.0).abs() < 1e-10);
    }

    #[test]
    fn act_act_isda_cross_year() {
        let from = Date::new(2024, 7, 1);
        let to = Date::new(2025, 7, 1);
        // 2024 is leap year (366 days), 2025 is not (365 days)
        // Days in 2024: Jul 1 to Jan 1 = 184 days out of 366
        // Days in 2025: Jan 1 to Jul 1 = 181 days out of 365
        let yf = DayCount::ActActIsda.year_fraction(from, to);
        let expected = 184.0 / 366.0 + 181.0 / 365.0;
        assert!((yf - expected).abs() < 1e-10);
    }

    #[test]
    fn accrue_thirty360_half_year_is_exact() {
        // 100 face * 5% * half year under 30/360 = exactly 2.5
        let base: Decimal = "5".parse().unwrap(); // 100 * 0.05
        let amount = DayCount::Thirty360.accrue(
            base,
            Date::new(2025, 1, 1),
            Date::new(2025, 7, 1),
        );
        assert_eq!(amount, "2.5".parse::<Decimal>().unwrap());
    }

    #[test]
    fn accrue_two_month_stub_is_exact() {
        // 100 * 6% * 60/360 = exactly 1 — would carry f64 noise if the
        // fraction were computed before the multiplication
        let base: Decimal = "6".parse().unwrap();
        let amount = DayCount::Thirty360.accrue(
            base,
            Date::new(2025, 1, 1),
            Date::new(2025, 3, 1),
        );
        assert_eq!(amount, Decimal::from(1));
    }

    #[test]
    fn accrue_matches_year_fraction() {
        use rust_decimal::prelude::ToPrimitive;
        let from = Date::new(2024, 7, 1);
        let to = Date::new(2025, 7, 1);
        for dc in [
            DayCount::Act360,
            DayCount::Act365Fixed,
            DayCount::Thirty360,
            DayCount::ActActIsda,
        ] {
            let via_f64 = dc.year_fraction(from, to);
            let via_decimal = dc
                .accrue(Decimal::ONE, from, to)
                .to_f64()
                .unwrap();
            assert!(
                (via_f64 - via_decimal).abs() < 1e-12,
                "{:?}: {} vs {}",
                dc,
                via_f64,
                via_decimal
            );
        }
    }

    #[test]
    fn daycount_serde() {
        let dc = DayCount::Act360;
        let json = serde_json::to_string(&dc).unwrap();
        assert_eq!(json, "\"Act360\"");
        let dc2: DayCount = serde_json::from_str(&json).unwrap();
        assert_eq!(dc, dc2);
    }

    #[test]
    fn compounding_continuous_roundtrip() {
        let rate = 0.05;
        let yf = 2.0;
        let df = Compounding::Continuous.df(rate, yf);
        let r2 = Compounding::Continuous.rate_from_df(df, yf);
        assert!((r2 - rate).abs() < 1e-12);
    }

    #[test]
    fn compounding_simple_roundtrip() {
        let rate = 0.05;
        let yf = 0.5;
        let df = Compounding::Simple.df(rate, yf);
        // df = 1 / (1 + 0.05 * 0.5) = 1 / 1.025
        assert!((df - 1.0 / 1.025).abs() < 1e-12);
        let r2 = Compounding::Simple.rate_from_df(df, yf);
        assert!((r2 - rate).abs() < 1e-12);
    }

    #[test]
    fn compounding_convert() {
        let cont_rate = 0.05;
        let yf = 1.0;
        let annual_rate = Compounding::Continuous.convert(cont_rate, yf, Compounding::Annual);
        // exp(0.05) = (1 + r_annual), so r_annual = exp(0.05) - 1
        let expected = (0.05_f64).exp() - 1.0;
        assert!((annual_rate - expected).abs() < 1e-12);
    }

    #[test]
    fn compounding_serde() {
        let c = Compounding::SemiAnnual;
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(json, "\"SemiAnnual\"");
        let c2: Compounding = serde_json::from_str(&json).unwrap();
        assert_eq!(c, c2);
    }
}

use serde::{Deserialize, Serialize};

use crate::core;
use crate::dates::Date;
use crate::dates::daycount::{Compounding, DayCount};

/// Discount curve: maps future dates to discount factors.
///
/// Internally stores continuously compounded yields at pillar points
/// with linear interpolation on `r * t` (where `t` is the year fraction
/// from the base date under the curve's day count convention).
///
/// This single type serves all purposes: risk-free discounting, credit curves,
/// funding curves, borrow curves. The identity (whose curve, what purpose)
/// comes from how it is stored in market data.
#[derive(Clone, Debug, Serialize)]
#[derive(Deserialize)]
#[serde(from = "DiscountCurveRaw")]
pub struct DiscountCurve {
    /// Valuation date (anchor for year fractions).
    base_date: Date,
    /// Day count convention for computing year fractions.
    day_count: DayCount,
    /// Pillar points: (date, continuously_compounded_yield).
    /// Must be sorted by date. At least one pillar required.
    pillars: Vec<(Date, f64)>,
    /// Pre-computed (year_fraction, r*t) at each pillar point.
    /// Avoids per-call allocation in the hot path.
    #[serde(skip)]
    pillar_rts: Vec<(f64, f64)>,
}

/// Serde helper: deserializes without pillar_rts, then recomputes it.
#[derive(Deserialize)]
struct DiscountCurveRaw {
    base_date: Date,
    day_count: DayCount,
    pillars: Vec<(Date, f64)>,
}

impl From<DiscountCurveRaw> for DiscountCurve {
    fn from(raw: DiscountCurveRaw) -> Self {
        let pillar_rts = Self::compute_pillar_rts(raw.base_date, raw.day_count, &raw.pillars);
        DiscountCurve {
            base_date: raw.base_date,
            day_count: raw.day_count,
            pillars: raw.pillars,
            pillar_rts,
        }
    }
}

impl DiscountCurve {
    /// Create a new discount curve.
    ///
    /// `pillars` must be non-empty and sorted by date.
    /// Each pillar is `(date, continuously_compounded_yield)`.
    pub fn new(
        base_date: Date,
        day_count: DayCount,
        pillars: Vec<(Date, f64)>,
    ) -> core::Result<DiscountCurve> {
        if pillars.is_empty() {
            return Err(core::Error::Curve("pillars must not be empty".to_string()));
        }
        for i in 1..pillars.len() {
            if pillars[i].0 <= pillars[i - 1].0 {
                return Err(core::Error::Curve(
                    "pillars must be sorted by date".to_string(),
                ));
            }
        }
        let pillar_rts = Self::compute_pillar_rts(base_date, day_count, &pillars);
        Ok(DiscountCurve {
            base_date,
            day_count,
            pillars,
            pillar_rts,
        })
    }

    /// Create a flat curve at a constant continuously compounded rate.
    pub fn flat(base_date: Date, day_count: DayCount, rate: f64) -> DiscountCurve {
        let far_date = base_date + 365 * 100; // ~100 years
        let pillars = vec![(base_date + 1, rate), (far_date, rate)];
        let pillar_rts = Self::compute_pillar_rts(base_date, day_count, &pillars);
        DiscountCurve {
            base_date,
            day_count,
            pillars,
            pillar_rts,
        }
    }

    fn compute_pillar_rts(base_date: Date, day_count: DayCount, pillars: &[(Date, f64)]) -> Vec<(f64, f64)> {
        pillars
            .iter()
            .map(|&(d, r)| {
                let t = day_count.year_fraction(base_date, d);
                (t, r * t)
            })
            .collect()
    }

    pub fn base_date(&self) -> Date {
        self.base_date
    }

    pub fn day_count(&self) -> DayCount {
        self.day_count
    }

    /// Year fraction from base date to `date`.
    fn year_fraction(&self, date: Date) -> f64 {
        self.day_count.year_fraction(self.base_date, date)
    }

    /// `r * t` at a given date, using linear interpolation on `r * t` values.
    fn rt(&self, date: Date) -> f64 {
        let t = self.year_fraction(date);
        if t <= 0.0 {
            return 0.0;
        }

        // Linear interpolation (flat extrapolation) on pre-computed pillar_rts
        if t <= self.pillar_rts[0].0 {
            // Before first pillar: use first rate
            return self.pillars[0].1 * t;
        }
        if t >= self.pillar_rts[self.pillar_rts.len() - 1].0 {
            // After last pillar: use last rate
            return self.pillars[self.pillars.len() - 1].1 * t;
        }

        // Find bracketing pillars
        for i in 1..self.pillar_rts.len() {
            if t <= self.pillar_rts[i].0 {
                let (t0, rt0) = self.pillar_rts[i - 1];
                let (t1, rt1) = self.pillar_rts[i];
                let w = (t - t0) / (t1 - t0);
                return rt0 + w * (rt1 - rt0);
            }
        }

        unreachable!()
    }

    /// Discount factor from `base_date` to `date`.
    /// df(T) = exp(-r(T) * T)
    pub fn df_to(&self, date: Date) -> f64 {
        (-self.rt(date)).exp()
    }

    /// Discount factor between two dates.
    /// df(T1, T2) = df(T2) / df(T1)
    pub fn df(&self, from: Date, to: Date) -> f64 {
        self.df_to(to) / self.df_to(from)
    }

    /// Continuously compounded zero rate to a given date.
    pub fn zero_rate(&self, date: Date) -> f64 {
        let t = self.year_fraction(date);
        if t.abs() < 1e-15 {
            // At base date, return the first pillar rate
            return self.pillars[0].1;
        }
        self.rt(date) / t
    }

    /// Forward rate between two dates (continuously compounded).
    pub fn forward_rate(&self, from: Date, to: Date) -> f64 {
        let t_from = self.year_fraction(from);
        let t_to = self.year_fraction(to);
        let dt = t_to - t_from;
        if dt.abs() < 1e-15 {
            return self.zero_rate(from);
        }
        (self.rt(to) - self.rt(from)) / dt
    }

    /// Convert the zero rate at a given date to a different compounding convention.
    pub fn rate_with_compounding(&self, date: Date, compounding: Compounding) -> f64 {
        let df = self.df_to(date);
        let t = self.year_fraction(date);
        compounding.rate_from_df(df, t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Date {
        Date::new(2025, 1, 1)
    }

    #[test]
    fn flat_curve() {
        let curve = DiscountCurve::flat(base(), DayCount::Act365Fixed, 0.05);
        let t1 = Date::new(2026, 1, 1);
        let df = curve.df_to(t1);
        let expected = (-0.05_f64).exp(); // ~1 year
        assert!((df - expected).abs() < 1e-6);
    }

    #[test]
    fn df_at_base_is_one() {
        let curve = DiscountCurve::flat(base(), DayCount::Act365Fixed, 0.05);
        let df = curve.df_to(base());
        assert!((df - 1.0).abs() < 1e-12);
    }

    #[test]
    fn df_between_dates() {
        let curve = DiscountCurve::flat(base(), DayCount::Act365Fixed, 0.05);
        let t1 = Date::new(2025, 7, 1);
        let t2 = Date::new(2026, 1, 1);
        let df_between = curve.df(t1, t2);
        // Should equal df_to(t2) / df_to(t1)
        let expected = curve.df_to(t2) / curve.df_to(t1);
        assert!((df_between - expected).abs() < 1e-12);
    }

    #[test]
    fn zero_rate_flat_curve() {
        let curve = DiscountCurve::flat(base(), DayCount::Act365Fixed, 0.05);
        let t1 = Date::new(2026, 1, 1);
        let rate = curve.zero_rate(t1);
        assert!((rate - 0.05).abs() < 1e-10);
    }

    #[test]
    fn forward_rate_flat_curve() {
        let curve = DiscountCurve::flat(base(), DayCount::Act365Fixed, 0.05);
        let t1 = Date::new(2025, 7, 1);
        let t2 = Date::new(2026, 7, 1);
        let fwd = curve.forward_rate(t1, t2);
        assert!((fwd - 0.05).abs() < 1e-10);
    }

    #[test]
    fn interpolated_curve() {
        let pillars = vec![
            (Date::new(2025, 7, 1), 0.04),  // 6M: 4%
            (Date::new(2026, 1, 1), 0.05),  // 1Y: 5%
            (Date::new(2027, 1, 1), 0.06),  // 2Y: 6%
        ];
        let curve = DiscountCurve::new(base(), DayCount::Act365Fixed, pillars).unwrap();

        // At pillar points, zero rate should match
        let r1 = curve.zero_rate(Date::new(2026, 1, 1));
        assert!((r1 - 0.05).abs() < 1e-10);

        let r2 = curve.zero_rate(Date::new(2027, 1, 1));
        assert!((r2 - 0.06).abs() < 1e-10);

        // Midpoint should be interpolated
        let r_mid = curve.zero_rate(Date::new(2026, 7, 2)); // ~1.5Y
        assert!(r_mid > 0.05 && r_mid < 0.06);
    }

    #[test]
    fn empty_pillars_rejected() {
        let result = DiscountCurve::new(base(), DayCount::Act365Fixed, vec![]);
        assert!(result.is_err());
    }

    #[test]
    fn unsorted_pillars_rejected() {
        let pillars = vec![
            (Date::new(2026, 1, 1), 0.05),
            (Date::new(2025, 7, 1), 0.04), // out of order
        ];
        let result = DiscountCurve::new(base(), DayCount::Act365Fixed, pillars);
        assert!(result.is_err());
    }

    #[test]
    fn rate_with_compounding() {
        let curve = DiscountCurve::flat(base(), DayCount::Act365Fixed, 0.05);
        let t1 = Date::new(2026, 1, 1);
        let annual_rate = curve.rate_with_compounding(t1, Compounding::Annual);
        // exp(0.05) = 1 + r_annual => r_annual = exp(0.05) - 1
        let expected = (0.05_f64).exp() - 1.0;
        assert!((annual_rate - expected).abs() < 1e-6);
    }

    #[test]
    fn curve_serde_roundtrip() {
        let pillars = vec![
            (Date::new(2025, 7, 1), 0.04),
            (Date::new(2026, 1, 1), 0.05),
        ];
        let curve = DiscountCurve::new(base(), DayCount::Act365Fixed, pillars).unwrap();
        let json = serde_json::to_string(&curve).unwrap();
        let curve2: DiscountCurve = serde_json::from_str(&json).unwrap();
        let test_date = Date::new(2025, 10, 1);
        assert!((curve.df_to(test_date) - curve2.df_to(test_date)).abs() < 1e-12);
    }
}

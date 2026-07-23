//! The publishable example book: a fully synthetic venue ("SYN") whose
//! settles, curves, and surfaces are pure functions of the date — no
//! licensed market data anywhere. Futures follow a deterministic
//! trend + sine path; option settles are Black-76 premiums off the same
//! smile the vols history carries, so the book is internally consistent.
//!
//! Regenerate the committed `market.json` + `vols.json` (byte-identical,
//! see `tests/gen_replay.rs`):
//!
//! ```sh
//! cargo run --example example-public
//! ```
//!
//! Price the book:
//!
//! ```sh
//! cargo run --bin qloxide-book -- --config examples/example-public/book.toml
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use qloxide::core;
use qloxide::curves::DiscountCurve;
use qloxide::dates::Date;
use qloxide::dates::daycount::DayCount;
use qloxide::generator::{self, DaySession, GenParams, MarketSource};
use qloxide::instruments::{FinancialInstrument, PutOrCall};
use qloxide::market_data::VolSurface;
use qloxide::pricing::black76::{Black76Params, black76_price};
use qloxide::trades::Deal;

/// Flat continuously compounded USD rate.
const RATE: f64 = 0.04;

fn from_day() -> Date {
    Date::new(2026, 7, 6)
}

fn through_day() -> Date {
    Date::new(2026, 7, 17)
}

/// Both options expire 2026-08-07, after the span end — every day
/// carries live settles and a surface.
fn option_expiry() -> Date {
    Date::new(2026, 8, 7)
}

/// Quote to the venue tick (0.01).
fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

fn day_index(date: Date) -> f64 {
    (date - from_day()) as f64
}

/// Sep-26 future: gentle uptrend around 101 with a sine wiggle.
fn u26_settle(date: Date) -> f64 {
    let n = day_index(date);
    round2(101.0 + 0.15 * n + 2.0 * libm::sin(0.7 * n))
}

/// Dec-26 future: its own path, holding a small contango to Sep.
fn z26_settle(date: Date) -> f64 {
    let n = day_index(date);
    round2(102.6 + 0.12 * n + 1.6 * libm::sin(0.6 * n + 1.0))
}

/// Smile in log-moneyness `m = ln(K/F)`: mild skew + smile around a
/// base vol that decays slightly over the span.
fn smile_vol(date: Date, log_m: f64) -> f64 {
    0.32 - 0.002 * day_index(date) + 0.05 * log_m + 0.35 * log_m * log_m
}

fn year_fraction_to_expiry(date: Date) -> f64 {
    (option_expiry() - date) as f64 / 365.0
}

/// Option settle: Black-76 off the U26 settle and the smile vol at the
/// option's own log-moneyness — the same surface the vols history carries.
fn option_settle(date: Date, strike: f64, put_or_call: PutOrCall) -> core::Result<f64> {
    let f = u26_settle(date);
    let params = Black76Params {
        f,
        k: strike,
        t: year_fraction_to_expiry(date),
        r: RATE,
        sigma: smile_vol(date, libm::log(strike / f)),
    };
    Ok(round2(black76_price(params, put_or_call)?))
}

struct SynVenue;

struct SynSession;

impl MarketSource for SynVenue {
    type Session<'s> = SynSession;

    fn open(&self, _from: Date, _through: Date) -> core::Result<SynSession> {
        Ok(SynSession)
    }

    fn source_stamp(&self) -> String {
        "SYNTHETIC".into()
    }

    fn generator_stamp(&self) -> String {
        "qloxide example-public (black76 synthetic)".into()
    }
}

impl DaySession for SynSession {
    fn is_trading_day(&mut self, date: Date) -> core::Result<bool> {
        Ok(!matches!(date.weekday(), 6 | 7))
    }

    fn market_price(&mut self, id: &str, date: Date) -> core::Result<Option<f64>> {
        Ok(match id {
            "SYN-U26" => Some(u26_settle(date)),
            "SYN-Z26" => Some(z26_settle(date)),
            _ => None,
        })
    }

    fn settle(&mut self, id: &str, date: Date) -> core::Result<Option<f64>> {
        Ok(match id {
            "SYN-U26" => Some(u26_settle(date)),
            "SYN-Z26" => Some(z26_settle(date)),
            "SYN-U26-C-105" => Some(option_settle(date, 105.0, PutOrCall::Call)?),
            "SYN-U26-P-95" => Some(option_settle(date, 95.0, PutOrCall::Put)?),
            _ => None,
        })
    }

    fn final_settle(&mut self, _id: &str, _date: Date) -> core::Result<Option<f64>> {
        // Nothing expires inside the span.
        Ok(None)
    }

    fn discount_curve(
        &mut self,
        currency: &str,
        date: Date,
    ) -> core::Result<Option<DiscountCurve>> {
        if currency != "USD" {
            return Ok(None);
        }
        let curve = DiscountCurve::new(
            date,
            DayCount::Act360,
            vec![(date + 1, RATE), (date + 36500, RATE)],
        )?;
        Ok(Some(curve))
    }

    fn vol_surface(&mut self, underlying: &str, date: Date) -> core::Result<Option<VolSurface>> {
        if underlying != "SYN-U26" {
            return Ok(None);
        }
        let moneyness: Vec<f64> = (0..=20).map(|i| f64::from(i) * 0.025 - 0.25).collect();
        let vols = vec![moneyness.iter().map(|&m| smile_vol(date, m)).collect()];
        Ok(Some(VolSurface::Grid {
            tenors: vec![year_fraction_to_expiry(date)],
            moneyness,
            vols,
        }))
    }
}

fn main() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/example-public");
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap();
    let instruments: Vec<Arc<dyn FinancialInstrument>> =
        serde_json::from_str(&read("instruments.json")).unwrap();
    let deals: Vec<Deal> = serde_json::from_str(&read("deals.json")).unwrap();

    let params = GenParams {
        from: Some(from_day()),
        through: through_day(),
    };
    let generated = generator::generate(&SynVenue, &instruments, Some(&deals), &params).unwrap();
    for warning in &generated.warnings {
        eprintln!("warning: {warning}");
    }

    let market = generated.market.to_canonical_json().unwrap();
    std::fs::write(dir.join("market.json"), &market).unwrap();
    println!(
        "wrote market.json ({} trading days, {} skipped)",
        generated.market.records().len(),
        generated.market.skipped_days().len()
    );
    let vols = generated
        .vols
        .expect("cleared options imply a vols history");
    std::fs::write(dir.join("vols.json"), vols.to_canonical_json().unwrap()).unwrap();
    println!(
        "wrote vols.json ({} days with surfaces)",
        vols.records().len()
    );
}

//! A [`MarketSource`] that replays committed histories.
//!
//! Answers every session question from an existing market history (plus
//! its optional vols history), so the generic engine can be validated
//! without any external venue: `generate` over a replay of a book's
//! committed files must reproduce them byte-identically. The icedat
//! machine uses the same property as the driver-equivalence gate.

use crate::core;
use crate::curves::DiscountCurve;
use crate::dates::Date;
use crate::market_data::{MarketHistory, VolSurface};

use super::{DaySession, MarketSource};

/// Replay of a committed (market, vols) pair.
pub struct ReplaySource {
    market: MarketHistory,
    /// Market with the vols history merged back in — the full per-day
    /// picture the original generator produced before the §9 split.
    merged: MarketHistory,
}

impl ReplaySource {
    pub fn new(market: MarketHistory, vols: Option<MarketHistory>) -> core::Result<ReplaySource> {
        let mut merged = market.clone();
        if let Some(vols) = vols {
            merged.merge_days(vols)?;
        }
        merged.validate()?;
        Ok(ReplaySource { market, merged })
    }
}

impl MarketSource for ReplaySource {
    type Session<'s> = ReplaySession<'s>;

    fn open(&self, _from: Date, _through: Date) -> core::Result<ReplaySession<'_>> {
        Ok(ReplaySession { src: self })
    }

    fn source_stamp(&self) -> String {
        self.market.source().unwrap_or_default().to_string()
    }

    fn generator_stamp(&self) -> String {
        self.market.generator().unwrap_or_default().to_string()
    }
}

pub struct ReplaySession<'s> {
    src: &'s ReplaySource,
}

impl DaySession for ReplaySession<'_> {
    fn is_trading_day(&mut self, date: Date) -> core::Result<bool> {
        Ok(self.src.market.is_trading_day(date))
    }

    fn market_price(&mut self, id: &str, date: Date) -> core::Result<Option<f64>> {
        Ok(self.src.merged.day(date)?.market_price(id).ok())
    }

    fn settle(&mut self, id: &str, date: Date) -> core::Result<Option<f64>> {
        Ok(self.src.merged.day(date)?.settlement_price(id).ok())
    }

    fn final_settle(&mut self, id: &str, date: Date) -> core::Result<Option<f64>> {
        // The committed record already carries the frozen final on every
        // post-expiry day — replaying it is the pass-through.
        self.settle(id, date)
    }

    fn discount_curve(
        &mut self,
        currency: &str,
        date: Date,
    ) -> core::Result<Option<DiscountCurve>> {
        Ok(self
            .src
            .merged
            .day(date)?
            .discount_curve(currency)
            .ok()
            .cloned())
    }

    fn vol_surface(&mut self, underlying: &str, date: Date) -> core::Result<Option<VolSurface>> {
        Ok(self
            .src
            .merged
            .day(date)?
            .vol_surface(underlying)
            .ok()
            .cloned())
    }
}

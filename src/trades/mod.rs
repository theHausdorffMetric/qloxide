use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::dates::Timestamp;

/// Buy or sell direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BuySell {
    Buy,
    Sell,
}

/// An executed trade — an immutable event recording that a transaction occurred.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Deal {
    pub id: String,
    pub instrument_id: String,
    pub direction: BuySell,
    pub quantity: Decimal,
    pub price: Decimal,
    pub timestamp: Timestamp,
    pub counterparty: String,
    pub venue: String,
}

impl Deal {
    /// Signed quantity: positive for Buy, negative for Sell.
    pub fn signed_quantity(&self) -> Decimal {
        match self.direction {
            BuySell::Buy => self.quantity,
            BuySell::Sell => -self.quantity,
        }
    }
}

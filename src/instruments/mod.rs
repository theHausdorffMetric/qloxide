pub mod bond;
pub mod equity;
pub mod future;
pub mod fx;
pub mod option;
pub mod swap;
pub mod basket;

use serde::{Deserialize, Serialize};

use crate::dates::Date;
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
}

impl Settlement {
    pub fn new(venue: &str, session: &str, time: &str, timezone: &str) -> Settlement {
        Settlement {
            venue: venue.to_string(),
            session: session.to_string(),
            time: time.to_string(),
            timezone: timezone.to_string(),
        }
    }

    /// Default settlement for OTC instruments.
    pub fn otc() -> Settlement {
        Settlement {
            venue: "OTC".to_string(),
            session: "CLOSE".to_string(),
            time: "17:00".to_string(),
            timezone: "America/New_York".to_string(),
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

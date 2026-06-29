use serde::Deserialize;
use std::error::Error;
use std::fmt::{Display, Formatter};

pub mod asian;
pub mod bachelier;
pub mod gbsm;
pub mod mathutils;

/// The type of an option, either `Put` or `Call`.
#[derive(Copy, Clone, Debug, Deserialize)]
pub enum OptionType {
    Put,
    Call,
}

impl std::fmt::Display for OptionType {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match *self {
            OptionType::Put => write!(f, "Put"),
            OptionType::Call => write!(f, "Call"),
        }
    }
}

// An error struct representing the class of response errors.
// TODO refac to use this error
#[non_exhaustive]
#[derive(Debug)]
pub enum QlError {
    ModelDataError(String),
    MarketDataError(String),
    IvolNonConvergeance(String, mathutils::MathError),
}

impl Display for QlError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match &self {
            QlError::ModelDataError(e) => write!(f, "GBSM incorrect model data: {}", e,),
            QlError::MarketDataError(e) => write!(f, "GBSM incorrect market data `{}`", e,),
            QlError::IvolNonConvergeance(e, _) => {
                write!(f, "ivol non-convergeance `{}`", e,)
            }
        }
    }
}

impl Error for QlError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self {
            QlError::ModelDataError(_) => None,
            QlError::MarketDataError(_) => None,
            QlError::IvolNonConvergeance(_, e) => Some(e),
        }
    }
}

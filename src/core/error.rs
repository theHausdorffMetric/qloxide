use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("date: {0}")]
    Date(String),

    #[error("interpolation: {0}")]
    Interpolation(String),

    #[error("market data: {0}")]
    MarketData(String),

    #[error("instrument: {0}")]
    Instrument(String),

    #[error("model: {0}")]
    Model(String),

    #[error("pricer: {0}")]
    Pricer(String),

    #[error("curve: {0}")]
    Curve(String),

    #[error("cash flow: {0}")]
    CashFlow(String),

    #[error("negative forward variance from {from} to {to}")]
    NegativeForwardVariance { from: String, to: String },

    #[error(transparent)]
    Serialization(#[from] serde_json::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

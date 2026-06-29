use crate::{gbsm::GBSMData, mathutils::root_brent, OptionType, QlError};
use serde::Deserialize;
use std::fmt::{Display, Formatter};

/// Structure containing market data for asian  models
///
/// This is mainly used to describe a market quote and then deduce implied volatility.
/// Fields are the same as for the struct GBSMdata, except for price vs volatility.
#[derive(Copy, Clone, Debug, Deserialize)]
pub struct AsianMarketData {
    /// a variant of the OptionType enum specifying either put or call type
    option_type: OptionType,
    /// the price of the underlying asset. Typically the current spot price or
    /// the futures price in case of options on futures
    asset_price: f64,
    /// the strike price
    strike: f64,
    /// the time to to the end of the averaging period (typically in fractions of a year, e.g.  0.5 is half a year)
    time: f64,
    /// the time to to the beginning of the averaging period (typically in fractions of a year, e.g.  0.5 is half a year)
    tau: f64,
    /// the continuous-time risk-free interest rate
    r: f64,
    /// the cost of carry:
    /// * `b=r` - gives the Black-Scholes (1973) stock option model
    /// * `b=r-q` - gives the Merton (1973) stock option model with continuous dividend yield `q`
    /// * `b=0` - gives the Black (1976) futures option model
    /// * `b=r=0`  - gives the Asay (1982) margined futures option model
    /// * `b=r-rf` - gives the Garman and Kohlhagen (1983) currency option model
    b: f64,
    /// the price of the option
    price: f64,
}

impl std::fmt::Display for AsianMarketData {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        write!(
            f,
            "OptionType:{}, asset price:{}, strike:{}, time:{}, tau:{}, interest:{}, carry:{}, price:{}",
            self.option_type, self.asset_price, self.strike, self.time, self.tau, self.r, self.b, self.price
        )
    }
}

impl AsianMarketData {
    pub fn new(
        option_type: OptionType,
        asset_price: f64,
        strike: f64,
        time: f64,
        tau: f64,
        r: f64,
        b: f64,
        price: f64,
    ) -> Result<AsianMarketData, QlError> {
        if asset_price <= 0.0 {
            return Err(QlError::MarketDataError(format!(
                "asset_price {asset_price} must be positive"
            )));
        }
        if strike <= 0.0 {
            return Err(QlError::MarketDataError(format!(
                "strike {strike} must be positive"
            )));
        }
        if tau < 0.0 || tau > time {
            return Err(QlError::MarketDataError(format!(
                "0 < start_averging:{tau} < end_averaging:{time}"
            )));
        }
        if price < 0.0 {
            return Err(QlError::MarketDataError(format!(
                "price {price} must be non-negative"
            )));
        }
        match option_type {
            OptionType::Call => {
                if price < f64::max(asset_price - strike, 0.0) {
                    return Err(QlError::MarketDataError(format!(
                        "price {price} must be higher than inner value"
                    )));
                }
            }
            OptionType::Put => {
                if price < f64::max(strike - asset_price, 0.0) {
                    return Err(QlError::MarketDataError(format!(
                        "price {price} must be higher than inner value"
                    )));
                }
            }
        }
        Ok(AsianMarketData {
            option_type,
            asset_price,
            strike,
            time,
            tau,
            r,
            b,
            price,
        })
    }
    pub fn ivol_brent(
        &self,
        x1: f64,
        x2: f64,
        max_err: f64,
        max_iter: usize,
    ) -> Result<f64, QlError> {
        root_brent(
            x1,
            x2,
            &|sigma: f64| {
                AsianData::new(
                    self.option_type,
                    self.asset_price,
                    self.strike,
                    self.time,
                    self.tau,
                    self.r,
                    self.b,
                    sigma,
                )
                .unwrap()
                .price()
                    - self.price
            },
            max_err,
            max_iter,
        )
        .map_err(|source| QlError::IvolNonConvergeance("ivol not found".to_string(), source))
    }
}

/// Structure containing model parameters needed for pricing generalized Black Scholes Merton style models
#[derive(Copy, Clone, Debug, Deserialize)]
pub struct AsianData {
    // TODO add params to error msg
    /// a variant of the OptionType enum specifying either put or call type
    option_type: OptionType,
    /// the price of the underlying asset. Typically the current spot price or
    /// the futures price in case of options on futures
    asset_price: f64,
    /// the strike price
    strike: f64,
    /// the time to maturity (typically in fractions of a year, e.g.  0.5 is half a year)
    /// the time to to the end of the averaging period (typically in fractions of a year, e.g.  0.5 is half a year)
    time: f64,
    /// the time to to the beginning of the averaging period (typically in fractions of a year, e.g.  0.5 is half a year)
    tau: f64,
    /// the continuous-time risk-free interest rate
    r: f64,
    /// the cost of carry:
    /// * `b=r` - gives the Black-Scholes (1973) stock option model
    /// * `b=r-q` - gives the Merton (1973) stock option model with continuous dividend yield `q`
    /// * `b=0` - gives the Black (1976) futures option model
    /// * `b=r=0`  - gives the Asay (1982) margined futures option model
    /// * `b=r-rf` - gives the Garman and Kohlhagen (1983) currency option model
    b: f64,
    /// the volatility (in terms of 0.1 for 10% per time unit)
    sigma: f64,
}

impl Display for AsianData {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "GBSM OptionType:{}, asset price:{}, strike:{}, time:{}, interest:{}, carry:{}, volatility:{}",
            self.option_type, self.asset_price, self.strike, self.time, self.r, self.b, self.sigma
        )
    }
}

impl AsianData {
    pub fn new(
        option_type: OptionType,
        asset_price: f64,
        strike: f64,
        time: f64,
        tau: f64,
        r: f64,
        b: f64,
        sigma: f64,
    ) -> Result<AsianData, QlError> {
        if asset_price <= 0.0 {
            return Err(QlError::MarketDataError(format!(
                "asset_price {asset_price} must be positive"
            )));
        }
        if strike <= 0.0 {
            return Err(QlError::MarketDataError(format!(
                "strike {strike} must be positive"
            )));
        }
        if tau < 0.0 || tau > time {
            return Err(QlError::MarketDataError(format!(
                "0 < start_averging:{tau} < end_averaging:{time}"
            )));
        }
        if sigma < 0.0 {
            return Err(QlError::ModelDataError(format!(
                "volatility {sigma} must be non-negative"
            )));
        }
        Ok(AsianData {
            option_type,
            asset_price,
            strike,
            time,
            tau,
            r,
            b,
            sigma,
        })
    }
}

impl AsianData {
    pub fn tw_gbsm_pars(&self) -> (f64, f64) {
        let m1: f64;
        let m2: f64;
        if f64::abs(self.b) < 1e-10f64 {
            m1 = 1.0f64;
            m2 = (2.0f64 * f64::exp(self.sigma * self.sigma * self.time)
                - 2.0f64
                    * f64::exp(self.sigma * self.sigma * self.tau)
                    * (1.0f64 + self.sigma * self.sigma * (self.time - self.tau)))
                / (f64::powf(self.sigma, 4.0f64) * (self.time - self.tau) * (self.time - self.tau));
        } else {
            m1 = (f64::exp(self.b * self.time) - f64::exp(self.b * self.tau))
                / (self.b * (self.time - self.tau));
            m2 = 2.0f64 * f64::exp((2.0f64 * self.b + self.sigma * self.sigma) * self.time)
                / ((self.b + self.sigma * self.sigma)
                    * (2.0f64 * self.b + self.sigma * self.sigma)
                    * (self.time - self.tau)
                    * (self.time - self.tau))
                + (2.0f64 * f64::exp(self.tau * (2.0f64 * self.b + self.sigma * self.sigma)))
                    / (self.b * (self.time - self.tau) * (self.time - self.tau))
                    * (1.0f64 / (2.0f64 * self.b + self.sigma * self.sigma)
                        - f64::exp(self.b * (self.time - self.tau))
                            / (self.b + self.sigma * self.sigma));
        }
        let b_a = m1.ln() / self.time;
        let sigma_a = (m2.ln() / self.time - 2.0 * b_a).sqrt();
        (b_a, sigma_a)
    }
    pub fn price(&self) -> f64 {
        let (b_a, sigma_a) = self.tw_gbsm_pars();
        GBSMData::new(
            self.option_type,
            self.asset_price,
            self.strike,
            self.time,
            self.r,
            b_a,
            sigma_a,
        )
        .unwrap()
        .price()
    }
}

#[cfg(test)]
mod tests {
    use crate::asian::*;
    #[test]
    fn check_tw_option() {
        let result = AsianData::new(OptionType::Call, 100.0, 100.0, 1.0, 0.5, 0.0, 0.0, 0.3)
            .unwrap()
            .price();
        let actual = 9.752_229_386_128_114_f64;
        let difference = (result - actual).abs();
        assert!(difference < 1.0e-12,);
    }
    #[test]
    fn check_tw_ivol() {
        let tol_digits = 5;
        let actual = 0.314159265f64;
        let tprem = AsianData::new(
            OptionType::Call,
            100.0,
            100.0,
            1.0,
            11.0 / 12.0,
            0.0,
            0.0,
            actual,
        )
        .unwrap()
        .price();
        let result = AsianMarketData::new(
            OptionType::Call,
            100.0,
            100.0,
            1.0,
            11.0 / 12.0,
            0.0,
            0.0,
            tprem,
        )
        .unwrap()
        .ivol_brent(0.01, 10.0, f64::powi(10.0f64, -tol_digits), 10_000);
        println!("{result:?}");
        let difference = (result.unwrap() - actual).abs();
        assert!(difference < f64::powi(10.0f64, -tol_digits));
    }
}

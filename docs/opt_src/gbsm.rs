use crate::{
    mathutils::{norm_cdf, norm_pdf, root_brent, root_nr},
    OptionType, QlError,
};
use log::{trace, warn};
use serde::Deserialize;
use std::f64::consts::PI;
use std::fmt::Display;

/// Struture containing market data for generalized Black Scholes Merton style models
///
/// This is mainly used to describe a market quote and then deduce implied volatility.
/// Fields are the same as for the struct GBSMdata, except for price vs volatility.
#[derive(Copy, Clone, Debug, Deserialize)]
pub struct GBSMMarketData {
    /// a variant of the OptionType enum specifying either put or call type
    option_type: OptionType,
    /// the price of the underlying asset. Typically the current spot price or
    /// the futures price in case of options on futures
    asset_price: f64,
    /// the strike price
    strike: f64,
    /// the time to maturity (typically in fractions of a year, e.g.  0.5 is half a year)
    time: f64,
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

impl std::fmt::Display for GBSMMarketData {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "OptionType:{}, asset price:{}, strike:{}, time:{}, interest:{}, carry:{}, price:{}",
            self.option_type, self.asset_price, self.strike, self.time, self.r, self.b, self.price
        )
    }
}

impl GBSMMarketData {
    pub fn new(
        option_type: OptionType,
        asset_price: f64,
        strike: f64,
        time: f64,
        r: f64,
        b: f64,
        price: f64,
    ) -> Result<GBSMMarketData, QlError> {
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
        if time < 0.0 {
            return Err(QlError::MarketDataError(format!(
                "time {time} must be non-negative"
            )));
        }
        if price < 0.0 {
            return Err(QlError::MarketDataError(format!(
                "price {price} must be non-negative"
            )));
        }
        // PROBLEM this my idiotic idea overlooked interest rates
        // match option_type {
        //     OptionType::Call => {
        //         if price < f64::max(asset_price - strike, 0.0) {
        //             return Err(QlError::MarketDataError(format!(
        //                 "price {price} must be higher than inner value {asset_price}-{strike}={}",
        //                 asset_price - strike
        //             )));
        //         }
        //     }
        //     OptionType::Put => {
        //         if price < f64::max(strike - asset_price, 0.0) {
        //             return Err(QlError::MarketDataError(format!(
        //                 "price {price} must be higher than inner value"
        //             )));
        //         }
        //     }
        // }
        Ok(GBSMMarketData {
            option_type,
            asset_price,
            strike,
            time,
            r,
            b,
            price,
        })
    }
    pub fn into_gbsm_data(&self, sigma: f64) -> Result<GBSMData, QlError> {
        GBSMData::new(
            self.option_type,
            self.asset_price,
            self.strike,
            self.time,
            self.r,
            self.b,
            sigma,
        )
    }
    /// flip the option type between put and call and adjust the price
    /// uses put-call parity
    fn flip(&self) -> Self {
        let tmp = ((self.b - self.r) * self.time).exp() * self.asset_price
            - (-self.r * self.time).exp() * self.strike;
        match self.option_type {
            OptionType::Put => GBSMMarketData {
                option_type: OptionType::Call,
                asset_price: self.asset_price,
                strike: self.strike,
                time: self.time,
                r: self.r,
                b: self.b,
                price: self.price + tmp,
            },
            OptionType::Call => GBSMMarketData {
                option_type: OptionType::Put,
                asset_price: self.asset_price,
                strike: self.strike,
                time: self.time,
                r: self.r,
                b: self.b,
                price: self.price - tmp,
            },
        }
    }
    /// calculate the implied volatility with Brent
    ///
    /// # Arguments
    /// * `f64` - the left bound
    /// * `f64` - the rightbound
    /// * `f64` - the max error
    /// * `usize` - the mat nr iterations
    ///
    /// # Return Value
    /// * `f64` - the implied volatility
    /// # Examples
    /// ```
    /// use bitql::{{gbsm::GBSMMarketData}, OptionType};
    /// let optdat = GBSMMarketData::new(OptionType::Call,
    ///          100.0, 100.0, 0.01, 0.0, 1.0, 10.0).unwrap();
    /// println!("Implied volatility: {:?}", optdat.ivol_brent(0.0,1000.0,0.001,10000));
    /// ```
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
                GBSMData::new(
                    self.option_type,
                    self.asset_price,
                    self.strike,
                    self.time,
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
        .map_err(|source| {
            QlError::IvolNonConvergeance(format!("GBSM {} ivol not found", *self), source)
        })
    }
    /// calculate the implied volatility with Newton Raphston
    ///
    /// NOTE: to be deprecated as can lead to negative vola
    /// which panics some other functions
    /// # Return Value
    /// * `f64` - the implied volatility
    /// # Examples
    /// ```
    /// use bitql::{{gbsm::GBSMMarketData}, OptionType};
    /// let optdat = GBSMMarketData::new(OptionType::Call,
    ///          100.0, 100.0, 0.01, 0.0, 1.0, 10.0).unwrap();
    /// println!("Implied volatility: {:?}", optdat.ivol_nr(0.001,10000));
    /// ```
    pub fn ivol_nr(&self, acceptable_err: f64, max_iterations: usize) -> Result<f64, QlError> {
        warn!("TODO divergence leads to negative vols which panics in other functions");
        let mut guess = self.ivol_cw();
        if guess.is_nan() {
            guess = 0.5;
            warn!("TODO debug {:?} initial ivol guess problem.", *self);
        }
        let res = root_nr(
            &|sigma: f64| {
                GBSMData::new(
                    self.option_type,
                    self.asset_price,
                    self.strike,
                    self.time,
                    self.r,
                    self.b,
                    sigma,
                )
                .unwrap()
                .price()
                    - self.price
            },
            &|sigma: f64| {
                GBSMData::new(
                    self.option_type,
                    self.asset_price,
                    self.strike,
                    self.time,
                    self.r,
                    self.b,
                    sigma,
                )
                .unwrap()
                .vega()
            },
            guess,
            acceptable_err,
            max_iterations,
        )
        .map_err(|source| QlError::IvolNonConvergeance("ivol not found".to_string(), source));
        if res.is_err() {
            trace!("ivol start {guess} NR problem {:?}", res);
        }
        res
    }
    /// Corrado Miller approximate implied volatility
    ///
    /// Calculate an quadratic approximation to implied volatility from premium. Follows
    /// C.J. Corrado, T. W. Miller, Jr./Journal of Banking & Finance 20 (1996) 595-603.
    /// p559 eq. (10)
    ///
    pub fn ivol_cw(&self) -> f64 {
        let premium = match self.option_type {
            OptionType::Put => self.flip().price,
            OptionType::Call => self.price,
        };
        let s = self.asset_price * f64::exp((self.b - self.r) * self.time);
        let x = self.strike * f64::exp((-self.r) * self.time);
        f64::sqrt(2.0 * PI / self.time) / (s + x)
            * (premium - (s - x) / 2.0
                + f64::sqrt((premium - (s - x) / 2.0).powi(2) - (s - x).powi(2) / PI))
    }
}

/// Struture containing model parameters needed for pricing generalized Black Scholes Merton style models
#[derive(Copy, Clone, Debug, Deserialize)]
pub struct GBSMData {
    // TODO add params to error msg
    /// a variant of the OptionType enum specifying either put or call type
    option_type: OptionType,
    /// the price of the underlying asset. Typically the current spot price or
    /// the futures price in case of options on futures
    asset_price: f64,
    /// the strike price
    strike: f64,
    /// the time to maturity (typically in fractions of a year, e.g.  0.5 is half a year)
    time: f64,
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

impl Display for GBSMData {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "GBSM OptionType:{}, asset price:{}, strike:{}, time:{}, interest:{}, carry:{}, volatility:{}",
            self.option_type, self.asset_price, self.strike, self.time, self.r, self.b, self.sigma
        )
    }
}

impl GBSMData {
    pub fn new(
        option_type: OptionType,
        asset_price: f64,
        strike: f64,
        time: f64,
        r: f64,
        b: f64,
        sigma: f64,
    ) -> Result<GBSMData, QlError> {
        if asset_price <= 0.0 {
            return Err(QlError::ModelDataError(format!(
                "asset_price {asset_price} must be positive"
            )));
        }
        if strike <= 0.0 {
            return Err(QlError::ModelDataError(format!(
                "strike {strike} must be positive"
            )));
        }
        if time < 0.0 {
            return Err(QlError::ModelDataError(format!(
                "time {time} must be non-negative"
            )));
        }
        if sigma < 0.0 {
            return Err(QlError::ModelDataError(format!(
                "volatility {sigma} must be non-negative"
            )));
        }
        Ok(GBSMData {
            option_type,
            asset_price,
            strike,
            time,
            r,
            b,
            sigma,
        })
    }
    pub fn into_gbsm_market_data(&self, price: f64) -> Result<GBSMMarketData, QlError> {
        GBSMMarketData::new(
            self.option_type,
            self.asset_price,
            self.strike,
            self.time,
            self.r,
            self.b,
            price,
        )
    }
    /// calculate the intrinsic value given a settlement value
    pub fn intrinsic_value(&self, settle: f64) -> f64 {
        match self.option_type {
            OptionType::Call => f64::exp(-self.r * self.time) * f64::max(settle - self.strike, 0.0),
            OptionType::Put => f64::exp(-self.r * self.time) * f64::max(self.strike - settle, 0.0),
        }
    }
    /// generalized Black-Scholes-Merton option price
    ///
    /// The option price assuming the underlying process is a geometric brownian motion. The
    /// formula is taken from Haug (2007) *Option Pricing Formulas*.
    ///
    /// # Arguments
    /// * `gbsm_data` - a GBSMData structure containing model data
    ///
    /// # Return Value
    /// * `f64` - the option price
    /// # Examples
    /// ```
    /// use bitql::{gbsm::GBSMData, OptionType};
    /// println!("Call(asset_type=100,strike=100,time=1,r=0.01,b=0,sigma=0.3) = {}",
    /// GBSMData::new(OptionType::Call, 100.0, 100.0, 1.0, 0.01, 0.0, 0.3).unwrap().price());
    /// ```
    pub fn price(&self) -> f64 {
        // return intrinsic value if time or vola is 0
        if self.time * self.sigma < 1.0e-6 {
            return self.intrinsic_value(self.asset_price);
        }
        let d1 = (f64::ln(self.asset_price / self.strike)
            + (self.b + self.sigma * self.sigma / 2.0) * self.time)
            / (self.sigma * f64::sqrt(self.time));
        let d2 = d1 - self.sigma * f64::sqrt(self.time);
        match self.option_type {
            OptionType::Call => {
                self.asset_price * ((self.b - self.r) * self.time).exp() * norm_cdf(d1)
                    - self.strike * (-self.r * self.time).exp() * norm_cdf(d2)
            }
            OptionType::Put => {
                self.strike * (-self.r * self.time).exp() * norm_cdf(-d2)
                    - self.asset_price * ((self.b - self.r) * self.time).exp() * norm_cdf(-d1)
            }
        }
    }
    /// generalized Black-Scholes-Merton delta
    ///
    /// The derivative of the option value with respect to the asset price.
    /// The formula is taken from Haug (2007) *Option Pricing Formulas*.
    ///
    /// # Arguments
    /// * `gbsm_data` - a GBSMData structure containing model data
    ///
    /// # Return Value
    /// * `f64` - the option delta
    /// # Examples
    /// ```
    /// use bitql::{gbsm::GBSMData, OptionType};
    /// println!("Call(asset_type=100,strike=100,time=1,r=0.01,b=0,sigma=0.3) = {}",
    /// GBSMData::new(OptionType::Call, 100.0, 100.0, 1.0, 0.01, 0.0, 0.3).unwrap().delta());
    /// ```
    pub fn delta(&self) -> f64 {
        let d1 = (f64::ln(self.asset_price / self.strike)
            + (self.b + self.sigma * self.sigma / 2.0) * self.time)
            / (self.sigma * f64::sqrt(self.time));
        match self.option_type {
            OptionType::Call => f64::exp((self.b - self.r) * self.time) * norm_cdf(d1),
            OptionType::Put => f64::exp((self.b - self.r) * self.time) * (norm_cdf(d1) - 1.0),
        }
    }
    pub fn strike_delta(&self) -> f64 {
        let d1 = (f64::ln(self.asset_price / self.strike)
            + (self.b + self.sigma * self.sigma / 2.0) * self.time)
            / (self.sigma * f64::sqrt(self.time));
        let d2 = d1 - self.sigma * f64::sqrt(self.time);
        match self.option_type {
            OptionType::Call => f64::exp((-self.r) * self.time) * norm_cdf(d2),
            OptionType::Put => f64::exp((-self.r) * self.time) * norm_cdf(-d2),
        }
    }
    pub fn gamma(&self) -> f64 {
        let d1 = (f64::ln(self.asset_price / self.strike)
            + (self.b + self.sigma * self.sigma / 2.0) * self.time)
            / (self.sigma * f64::sqrt(self.time));
        f64::exp((self.b - self.r) * self.time) * norm_pdf(d1)
            / (self.asset_price * self.sigma * f64::sqrt(self.time))
    }
    // TODO test
    pub fn theta(&self) -> f64 {
        let d1 = (f64::ln(self.asset_price / self.strike)
            + (self.b + self.sigma * self.sigma / 2.0) * self.time)
            / (self.sigma * f64::sqrt(self.time));
        let d2 = d1 - self.sigma * f64::sqrt(self.time);
        let theta1 = -(self.asset_price
            * f64::exp((self.b - self.r) * self.time)
            * norm_pdf(d1)
            * self.sigma)
            / (2.0 * f64::sqrt(self.time));
        match self.option_type {
            OptionType::Call => {
                theta1
                    - (self.b - self.r)
                        * self.asset_price
                        * f64::exp((self.b - self.r) * self.time)
                        * norm_cdf(d1)
                    - self.r * self.strike * f64::exp(-self.r * self.time) * norm_cdf(d2)
            }
            OptionType::Put => {
                theta1
                    + (self.b - self.r)
                        * self.asset_price
                        * f64::exp((self.b - self.r) * self.time)
                        * norm_cdf(-d1)
                    + self.r * self.strike * f64::exp(-self.r * self.time) * norm_cdf(-d2)
            }
        }
    }
    pub fn vega(&self) -> f64 {
        let d1 = (f64::ln(self.asset_price / self.strike)
            + (self.b + self.sigma * self.sigma / 2.0) * self.time)
            / (self.sigma * f64::sqrt(self.time));
        self.asset_price
            * f64::exp((self.b - self.r) * self.time)
            * norm_pdf(d1)
            * f64::sqrt(self.time)
    }
}

#[cfg(test)]
mod tests {
    use crate::gbsm::{GBSMData, GBSMMarketData, OptionType};
    #[test]
    fn check_gbsm_option() {
        assert!(
            (GBSMData::new(OptionType::Call, 100.0, 100.0, 1.0, 0.01, 0.0, 0.3)
                .unwrap()
                .price()
                - 11.80489728393353f64)
                .abs()
                < 1e-12
        );
    }
    #[test]
    fn check_gbsm_delta() {
        assert!(
            (GBSMData::new(OptionType::Call, 105.0, 100.0, 0.5, 0.1, 0.0, 0.36)
                .unwrap()
                .delta()
                - 0.5946)
                .abs()
                < 1e-4
        );
        assert!(
            (GBSMData::new(OptionType::Put, 105.0, 100.0, 0.5, 0.1, 0.0, 0.36)
                .unwrap()
                .delta()
                + 0.3566)
                .abs()
                < 1e-4
        );
    }
    #[test]
    fn check_gbsm_gamma() {
        assert!(
            (GBSMData::new(OptionType::Call, 55.0, 60.0, 0.75, 0.105, 0.0695, 0.3)
                .unwrap()
                .gamma()
                - 0.02718)
                .abs()
                < 1e-4
        );
    }
    #[test]
    fn check_gbsm_vega() {
        let jnk = (10000.0
            * GBSMData::new(OptionType::Call, 55.0, 60.0, 0.75, 0.105, 0.0695, 0.3)
                .unwrap()
                .vega())
        .round()
            / 10000.0;
        assert_eq!(jnk, 18.5027f64);
    }
    #[test]
    fn check_flip() {
        let call_price = GBSMData::new(OptionType::Call, 55.0, 60.0, 0.75, 0.05, 0.0, 0.3)
            .unwrap()
            .price();
        let put_price = GBSMData::new(OptionType::Put, 55.0, 60.0, 0.75, 0.05, 0.0, 0.3)
            .unwrap()
            .price();
        let price_flip1 =
            GBSMMarketData::new(OptionType::Call, 55.0, 60.0, 0.75, 0.05, 0.0, call_price)
                .unwrap()
                .flip();
        assert!((price_flip1.price - put_price).abs() < 1e-12);
        let price_flip2 =
            GBSMMarketData::new(OptionType::Put, 55.0, 60.0, 0.75, 0.05, 0.0, put_price)
                .unwrap()
                .flip();
        assert!((price_flip2.price - call_price).abs() < 1e-12);
    }
    // #[test]
    // fn check_gbsm_cw_approx() {
    //     let call_value = gbsm_option(OptionType::Call, 55.0, 60.0, 0.75, 0.105, 0.0695, 0.3);
    //     let approx_vol = gbsm_cw_approx_vol(
    //         call_value,
    //         OptionType::Call,
    //         55.0,
    //         60.0,
    //         0.75,
    //         0.105,
    //         0.0695,
    //     );
    //     assert_abs_diff_eq!(0.3, approx_vol, epsilon = 0.01);
    // }
    //     #[test]
    //     fn check_gbsm_ivol() {
    //         let tol_digits = 8;
    //         let actual = 0.314159265f64;
    //         let tprem = gbsm_option(OptionType::Call, 100.0, 100.0, 1.0, 0.0, 0.0, actual);
    //         let result = gbsm_ivol(
    //             tprem,
    //             OptionType::Call,
    //             100.0,
    //             100.0,
    //             1.0,
    //             0.0,
    //             0.0,
    //             f64::powi(10.0f64, -tol_digits),
    //             1000000,
    //         )
    //         .unwrap();
    //         let difference = (result - actual).abs();
    //         assert!(
    //             difference < f64::powi(10.0f64, -tol_digits),
    //             "result = {}, actual = {}",
    //             result,
    //             actual
    //         );
    //     }
}

use crate::{
    mathutils::{norm_cdf, norm_pdf, root_brent, root_nr},
    OptionType, QlError,
};
use ::std::fmt::Display;
use serde::Deserialize;

/// Struture containing market data to derive model parameters
#[derive(Copy, Clone, Debug, Deserialize)]
pub struct BachelierMarketData {
    /// a variant of the OptionType enum specifying either put or call type
    option_type: OptionType,
    /// the forward price of the underlying asset.
    forward: f64,
    /// the strike price
    strike: f64,
    /// the time to maturity (typically in fractions of a year, e.g. 0.5 is half a year)
    time: f64,
    /// the continuous-time risk-free interest rate
    r: f64,
    /// the market price of the option
    price: f64,
}

impl std::fmt::Display for BachelierMarketData {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "OptionType:{}, asset price:{}, strike:{}, time:{}, interest:{}, price: {}",
            self.option_type, self.forward, self.strike, self.time, self.r, self.price,
        )
    }
}

impl BachelierMarketData {
    pub fn new(
        option_type: OptionType,
        forward: f64,
        strike: f64,
        time: f64,
        r: f64,
        price: f64,
    ) -> Result<BachelierMarketData, QlError> {
        if time < 0.0 {
            return Err(QlError::ModelDataError(format!(
                "time={} cannot be negative.",
                time
            )));
        };
        Ok(BachelierMarketData {
            option_type,
            forward,
            strike,
            time,
            r,
            price,
        })
    }
    pub fn ivol(&self) -> f64 {
        let theta: f64 = match self.option_type {
            OptionType::Call => 1.0,
            OptionType::Put => -1.0,
        };
        let price_undiscounted = f64::exp(self.r * self.time) * self.price;
        let v: f64 = f64::abs(self.forward - self.strike)
            / (2.0 * price_undiscounted - theta * (self.forward - self.strike));
        let eta: f64 = if v.abs() < 0.0001 {
            1.0 / (1.0
                + f64::powf(v, 2.0) / 3.0
                + f64::powf(v, 4.0) / 5.0
                + f64::powf(v, 6.0) / 7.0)
        } else {
            v / f64::atanh(v)
        };
        let a0: f64 = 3.99496_16873_45134 * f64::powf(10.0, -1.0);
        let a1: f64 = 2.10096_07950_68497 * f64::powf(10.0, 1.0);
        let a2: f64 = 4.98034_02178_55084 * f64::powf(10.0, 1.0);
        let a3: f64 = 5.98876_11026_90991 * f64::powf(10.0, 2.0);
        let a4: f64 = 1.84848_96954_37094 * f64::powf(10.0, 3.0);
        let a5: f64 = 6.10632_24078_67059 * f64::powf(10.0, 3.0);
        let a6: f64 = 2.49341_52853_49361 * f64::powf(10.0, 4.0);
        let a7: f64 = 1.26645_80513_48246 * f64::powf(10.0, 4.0);
        let b1: f64 = 4.99053_41535_89422 * f64::powf(10.0, 1.0);
        let b2: f64 = 3.09357_39367_43112 * f64::powf(10.0, 1.0);
        let b3: f64 = 1.49510_50083_10999 * f64::powf(10.0, 3.0);
        let b4: f64 = 1.32361_45378_99738 * f64::powf(10.0, 3.0);
        let b5: f64 = 1.59891_96976_79745 * f64::powf(10.0, 4.0);
        let b6: f64 = 2.39200_88917_20782 * f64::powf(10.0, 4.0);
        let b7: f64 = 3.60881_71083_75034 * f64::powf(10.0, 3.0);
        let b8: f64 = -2.06771_94864_00926 * f64::powf(10.0, 2.0);
        let b9: f64 = 1.17424_05993_06013 * f64::powf(10.0, 1.0);
        let h = eta.sqrt()
            * (a0
                + a1 * eta
                + a2 * f64::powf(eta, 2.0)
                + a3 * f64::powf(eta, 3.0)
                + a4 * f64::powf(eta, 4.0)
                + a5 * f64::powf(eta, 5.0)
                + a6 * f64::powf(eta, 6.0)
                + a7 * f64::powf(eta, 7.0))
            / (1.0
                + b1 * eta
                + b2 * f64::powf(eta, 2.0)
                + b3 * f64::powf(eta, 3.0)
                + b4 * f64::powf(eta, 4.0)
                + b5 * f64::powf(eta, 5.0)
                + b6 * f64::powf(eta, 6.0)
                + b7 * f64::powf(eta, 7.0)
                + b8 * f64::powf(eta, 8.0)
                + b9 * f64::powf(eta, 9.0));
        (std::f64::consts::PI / 2.0 / self.time).sqrt()
            * (2.0 * price_undiscounted - theta * (self.forward - self.strike))
            * h
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
                BachelierData::new(
                    self.option_type,
                    self.forward,
                    self.strike,
                    self.time,
                    self.r,
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
            QlError::IvolNonConvergeance("Bachelier ivol not found".to_string(), source)
        })
    }
    pub fn ivol_nr(&self, start_value: f64, max_err: f64, max_iter: usize) -> Result<f64, QlError> {
        root_nr(
            &|sigma: f64| {
                BachelierData::new(
                    self.option_type,
                    self.forward,
                    self.strike,
                    self.time,
                    self.r,
                    sigma,
                )
                .unwrap()
                .price()
                    - self.price
            },
            &|sigma: f64| {
                BachelierData::new(
                    self.option_type,
                    self.forward,
                    self.strike,
                    self.time,
                    self.r,
                    sigma,
                )
                .unwrap()
                .vega()
            },
            start_value,
            max_err,
            max_iter,
        )
        .map_err(|source| {
            QlError::IvolNonConvergeance("Bachelier ivol not found".to_string(), source)
        })
    }
}

/// Struture containing parameters needed for pricing
#[derive(Copy, Clone, Debug, Deserialize)]
pub struct BachelierData {
    /// a variant of the OptionType enum specifying either put or call type
    option_type: OptionType,
    /// the forward price of the underlying asset.
    forward: f64,
    /// the strike price
    strike: f64,
    /// the time to maturity (typically in fractions of a year, e.g. 0.5 is half a year)
    time: f64,
    /// the continuous-time risk-free interest rate
    r: f64,
    /// the sd
    sd: f64,
}

impl Display for BachelierData {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "Bachelier OptionType:{}, asset price:{}, strike:{}, time:{}, interest:{}, sd:{}",
            self.option_type, self.forward, self.strike, self.time, self.r, self.sd
        )
    }
}

impl BachelierData {
    /// calculate the intrinsic value
    pub fn intrinsic_value(&self) -> f64 {
        match self.option_type {
            OptionType::Call => {
                f64::exp(-self.r * self.time) * f64::max(self.forward - self.strike, 0.0)
            }
            OptionType::Put => {
                f64::exp(-self.r * self.time) * f64::max(self.strike - self.forward, 0.0)
            }
        }
    }
    // intrinsic delta
    pub fn intrinsic_delta(&self) -> f64 {
        let m = self.forward - self.strike;
        match self.option_type {
            OptionType::Call => {
                if m > 0.0 {
                    f64::exp(-self.r * self.time) * 1.0
                } else if m < 0.0 {
                    f64::exp(-self.r * self.time) * 0.0
                } else {
                    f64::NAN
                }
            }
            OptionType::Put => {
                if m > 0.0 {
                    f64::exp(-self.r * self.time) * 0.0
                } else if m < 0.0 {
                    f64::exp(-self.r * self.time) * -1.0
                } else {
                    f64::NAN
                }
            }
        }
    }
    pub fn new(
        option_type: OptionType,
        forward: f64,
        strike: f64,
        time: f64,
        r: f64,
        sd: f64,
    ) -> Result<BachelierData, QlError> {
        if time < 0.0 {
            return Err(QlError::ModelDataError(format!(
                "time={} cannot be negative.",
                time
            )));
        };
        if sd < 0.0 {
            return Err(QlError::ModelDataError(format!(
                "sd={} cannot be negative.",
                sd
            )));
        };
        Ok(BachelierData {
            option_type,
            forward,
            strike,
            time,
            r,
            sd,
        })
    }
    pub fn price(&self) -> f64 {
        let d = (self.forward - self.strike) / self.sd / f64::sqrt(self.time);
        if d.is_nan() || d.is_infinite() {
            self.intrinsic_value()
        } else {
            match self.option_type {
                OptionType::Call => {
                    f64::exp(-self.r * self.time)
                        * self.sd
                        * f64::sqrt(self.time)
                        * (d * norm_cdf(d) + norm_pdf(d))
                }
                OptionType::Put => {
                    f64::exp(-self.r * self.time)
                        * self.sd
                        * f64::sqrt(self.time)
                        * (-d * norm_cdf(-d) + norm_pdf(d))
                }
            }
        }
    }
    pub fn delta(&self) -> f64 {
        let d = (self.forward - self.strike) / self.sd / f64::sqrt(self.time);
        if d.is_nan() || d.is_infinite() {
            self.intrinsic_delta()
        } else {
            match self.option_type {
                OptionType::Call => f64::exp(-self.r * self.time) * norm_cdf(d),
                OptionType::Put => f64::exp(-self.r * self.time) * (norm_cdf(d) - 1.0),
            }
        }
    }
    pub fn gamma(&self) -> f64 {
        let d = (self.forward - self.strike) / self.sd / f64::sqrt(self.time);
        if d.is_nan() || d.is_infinite() {
            0.0
        } else {
            f64::exp(-self.r * self.time) * norm_pdf(d) / self.sd / f64::sqrt(self.time)
        }
    }
    pub fn theta(&self) -> f64 {
        let d = (self.forward - self.strike) / self.sd / f64::sqrt(self.time);
        if d.is_nan() || d.is_infinite() {
            0.0
        } else {
            -self.r * self.price()
                + 0.5 * f64::exp(-self.r * self.time) * self.sd * norm_pdf(d) / f64::sqrt(self.time)
        }
    }
    pub fn vega(self) -> f64 {
        let d = (self.forward - self.strike) / self.sd / f64::sqrt(self.time);
        if d.is_nan() || d.is_infinite() {
            0.0
        } else {
            f64::exp(-self.r * self.time) * norm_pdf(d) * f64::sqrt(self.time)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn check_1() {
        let tmp = BachelierData::new(OptionType::Call, 100.0, 100.0, 0.5, 0.01, 40.0).unwrap();
        assert!((tmp.price() - 11.227_513_525_210_801) <= f64::EPSILON);
    }
    #[test]
    fn check_negtime() {
        assert!(BachelierData::new(OptionType::Call, 100.0, 100.0, -0.5, 0.01, 40.0).is_err());
    }
    #[test]
    fn check_negsd() {
        assert!(BachelierData::new(OptionType::Call, 100.0, 100.0, 0.5, 0.01, -40.0).is_err());
    }
    #[test]
    fn check_zerot1() {
        let tmp = BachelierData::new(OptionType::Call, 100.0, 100.0, 0.0, 0.01, 40.0).unwrap();
        assert!((tmp.price()).abs() < f64::EPSILON);
    }
    #[test]
    fn check_zerot2() {
        let tmp = BachelierData::new(OptionType::Call, 100.0, 90.0, 0.0, 0.01, 40.0).unwrap();
        assert!((tmp.price() - 10.0).abs() < f64::EPSILON);
    }
    #[test]
    fn check_zerosd1() {
        let tmp = BachelierData::new(OptionType::Call, 100.0, 100.0, 0.5, 0.01, 0.0).unwrap();
        assert!(tmp.price().abs() < f64::EPSILON);
    }
    #[test]
    fn check_zerosd2() {
        let tmp = BachelierData::new(OptionType::Call, 100.0, 90.0, 0.5, 0.01, 0.0).unwrap();
        assert!((tmp.price() - f64::exp(-0.01 * 0.5) * 10.0).abs() < f64::EPSILON);
    }
    #[test]
    fn check_zerotimesd1() {
        let tmp = BachelierData::new(OptionType::Call, 100.0, 100.0, 0.0, 0.01, 0.0).unwrap();
        assert!(tmp.price().abs() < f64::EPSILON);
    }
    #[test]
    fn check_zerotimesd2() {
        let tmp = BachelierData::new(OptionType::Call, 100.0, 90.0, 0.0, 0.01, 0.0).unwrap();
        assert!((tmp.price() - 10.0).abs() < f64::EPSILON);
    }
}

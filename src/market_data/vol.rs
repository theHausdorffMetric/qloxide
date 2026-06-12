use serde::{Deserialize, Serialize};

/// Volatility surface — vol as a function of tenor and moneyness.
///
/// `tenor` is time to expiry in years; `moneyness` is log-moneyness
/// `ln(K/F)`. The surface variant also carries the model: a `Flat`
/// (lognormal) vol implies Black76. A future `FlatBachelier` (normal vol)
/// variant will imply Bachelier — no separate model parameter exists.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum VolSurface {
    /// Constant lognormal vol regardless of tenor/moneyness.
    Flat { vol: f64 },
    // Future variants:
    // FlatBachelier { vol } — normal model
    // Grid { tenors, strikes, vols } — interpolated surface
    // Sabr { alpha, beta, rho, nu } — parametric
}

impl VolSurface {
    /// Volatility at the given tenor (years) and log-moneyness ln(K/F).
    pub fn vol(&self, _tenor: f64, _moneyness: f64) -> f64 {
        match self {
            VolSurface::Flat { vol } => *vol,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_ignores_tenor_and_moneyness() {
        let s = VolSurface::Flat { vol: 0.30 };
        assert_eq!(s.vol(0.5, 0.0), 0.30);
        assert_eq!(s.vol(2.0, -0.15), 0.30);
    }

    #[test]
    fn serde_roundtrip() {
        let s = VolSurface::Flat { vol: 0.30 };
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, r#"{"type":"Flat","vol":0.3}"#);
        let s2: VolSurface = serde_json::from_str(&json).unwrap();
        assert_eq!(s2.vol(1.0, 0.0), 0.30);
    }
}

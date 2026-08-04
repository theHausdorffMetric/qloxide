use std::sync::Arc;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::cashflows::CashFlow;
use crate::dates::Date;
use crate::instruments::{FinancialInstrument, Settlement};
use crate::reference_data::Currency;

/// An unconditional cash payment: a currency amount on a date, as a
/// bookable position.
///
/// This is [`CashFlow`] promoted to instrument rank — with identity and a
/// counterparty — so the terminal state of other instruments (a cash-settled
/// future at expiry, an exercised option, one leg of an FX forward) can be
/// held in a portfolio rather than only computed in passing.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Payment {
    pub id: String,
    pub credit_id: String,
    /// Cash amount (positive = receive, negative = pay).
    pub amount: Decimal,
    pub currency: Arc<Currency>,
    pub settlement: Settlement,
    /// Payment date.
    pub pay_date: Date,
}

impl Payment {
    pub fn new(
        id: &str,
        credit_id: &str,
        amount: Decimal,
        currency: Arc<Currency>,
        settlement: Settlement,
        pay_date: Date,
    ) -> Payment {
        Payment {
            id: id.to_string(),
            credit_id: credit_id.to_string(),
            amount,
            currency,
            settlement,
            pay_date,
        }
    }

    /// The payment as a plain [`CashFlow`] value.
    pub fn cash_flow(&self) -> CashFlow {
        CashFlow::new(self.amount, Arc::clone(&self.currency), self.pay_date)
    }
}

#[typetag::serde]
impl FinancialInstrument for Payment {
    fn id(&self) -> &str {
        &self.id
    }

    fn currency(&self) -> &Currency {
        &self.currency
    }

    fn settlement(&self) -> &Settlement {
        &self.settlement
    }

    fn maturity(&self) -> Option<Date> {
        Some(self.pay_date)
    }

    fn instrument_type(&self) -> &str {
        "Payment"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dates::daycount::DayCount;
    use crate::dates::rules::DateRule;

    fn usd() -> Arc<Currency> {
        Arc::new(Currency::new("USD", DateRule::Null, DayCount::Act360))
    }

    fn test_payment() -> Payment {
        Payment::new(
            "PAY-1",
            "ACME",
            Decimal::from(1_000_000),
            usd(),
            Settlement::otc(),
            Date::new(2026, 12, 15),
        )
    }

    #[test]
    fn trait_accessors() {
        let p = test_payment();
        assert_eq!(p.id(), "PAY-1");
        assert_eq!(p.currency().id, "USD");
        assert_eq!(p.maturity(), Some(Date::new(2026, 12, 15)));
        assert_eq!(p.clearing(), None);
        assert_eq!(p.instrument_type(), "Payment");
    }

    #[test]
    fn serde_roundtrip_as_trait_object() {
        let p: Box<dyn FinancialInstrument> = Box::new(test_payment());
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"type\":\"Payment\""));

        let back: Box<dyn FinancialInstrument> = serde_json::from_str(&json).unwrap();
        let q = back.as_any().downcast_ref::<Payment>().unwrap();
        assert_eq!(q.id, "PAY-1");
        assert_eq!(q.credit_id, "ACME");
        assert_eq!(q.amount, Decimal::from(1_000_000));
        assert_eq!(q.pay_date, Date::new(2026, 12, 15));
    }

    #[test]
    fn cash_flow_view_matches_fields() {
        let p = test_payment();
        let cf = p.cash_flow();
        assert_eq!(cf.amount, p.amount);
        assert_eq!(cf.currency.id, "USD");
        assert_eq!(cf.pay_date, p.pay_date);
    }
}

use std::sync::Arc;

use qloxide::dates::Date;
use qloxide::dates::daycount::DayCount;
use qloxide::dates::rules::DateRule;
use qloxide::cashflows::Frequency;
use qloxide::instruments::*;
use qloxide::reference_data::Currency;

fn usd() -> Arc<Currency> {
    Arc::new(Currency::new("USD", DateRule::Null, DayCount::Act360))
}

fn gbp() -> Arc<Currency> {
    Arc::new(Currency::new("GBP", DateRule::Null, DayCount::Act365Fixed))
}

fn ice_settle() -> Settlement {
    Settlement::new("ICE", "SETTLE", "19:30", "Europe/London")
}

// ── Equity ───────────────────────────────────────────────────────────

#[test]
fn equity_serde_roundtrip() {
    let equity = Equity::new("AAPL", "APPLE_INC", usd(), Settlement::otc());
    let inst: Arc<dyn FinancialInstrument> = Arc::new(equity);

    let json = serde_json::to_string_pretty(&inst).unwrap();
    assert!(json.contains("\"type\": \"Equity\""));
    assert!(json.contains("\"AAPL\""));

    let deserialized: Arc<dyn FinancialInstrument> = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.id(), "AAPL");
    assert_eq!(deserialized.instrument_type(), "Equity");
    assert_eq!(deserialized.currency().id, "USD");
    assert!(deserialized.maturity().is_none());
}

// ── Future ───────────────────────────────────────────────────────────

#[test]
fn future_serde_roundtrip() {
    let future = Future::new(
        "ICE-BRN-Jun25",
        "Brent",
        usd(),
        ice_settle(),
        Date::new(2025, 6, 14),
        1000.0,
        0.01,
    );
    let inst: Arc<dyn FinancialInstrument> = Arc::new(future);

    let json = serde_json::to_string_pretty(&inst).unwrap();
    assert!(json.contains("\"type\": \"Future\""));

    let deserialized: Arc<dyn FinancialInstrument> = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.id(), "ICE-BRN-Jun25");
    assert_eq!(deserialized.maturity(), Some(Date::new(2025, 6, 14)));
}

// ── EuropeanOption ───────────────────────────────────────────────────

#[test]
fn option_serde_roundtrip() {
    let option = EuropeanOption::new(
        "AAPL-C-190-Jun25",
        "AAPL",
        "OCC",
        usd(),
        Settlement::otc(),
        Date::new(2025, 6, 20),
        190.0,
        PutOrCall::Call,
        OptionSettlement::Cash,
        Date::new(2025, 6, 22),
    );
    let inst: Arc<dyn FinancialInstrument> = Arc::new(option);

    let json = serde_json::to_string_pretty(&inst).unwrap();
    assert!(json.contains("\"type\": \"EuropeanOption\""));
    assert!(json.contains("\"Call\""));

    let deserialized: Arc<dyn FinancialInstrument> = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.id(), "AAPL-C-190-Jun25");
}

#[test]
fn option_intrinsic_value() {
    let call = EuropeanOption::new(
        "C", "UND", "CR", usd(), Settlement::otc(),
        Date::new(2025, 6, 20), 100.0, PutOrCall::Call,
        OptionSettlement::Cash, Date::new(2025, 6, 22),
    );
    assert_eq!(call.intrinsic(110.0), 10.0);
    assert_eq!(call.intrinsic(90.0), 0.0);

    let put = EuropeanOption::new(
        "P", "UND", "CR", usd(), Settlement::otc(),
        Date::new(2025, 6, 20), 100.0, PutOrCall::Put,
        OptionSettlement::Cash, Date::new(2025, 6, 22),
    );
    assert_eq!(put.intrinsic(90.0), 10.0);
    assert_eq!(put.intrinsic(110.0), 0.0);
}

// ── Bond ─────────────────────────────────────────────────────────────

#[test]
fn bond_serde_roundtrip() {
    let bond = Bond::new(
        "US-TBOND-5Y",
        "US_GOVT",
        usd(),
        Settlement::otc(),
        Date::new(2025, 1, 15),
        Date::new(2030, 1, 15),
        1_000_000.0,
        0.045,
        DayCount::Thirty360,
        2,
    );
    let inst: Arc<dyn FinancialInstrument> = Arc::new(bond.clone());

    let json = serde_json::to_string_pretty(&inst).unwrap();
    assert!(json.contains("\"type\": \"Bond\""));

    let deserialized: Arc<dyn FinancialInstrument> = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.id(), "US-TBOND-5Y");
    assert_eq!(deserialized.maturity(), Some(Date::new(2030, 1, 15)));

    // Coupon amount: 1M * 4.5% / 2 = 22500
    assert_eq!(bond.coupon_amount(), 22_500.0);
}

// ── Swap ─────────────────────────────────────────────────────────────

#[test]
fn swap_serde_roundtrip() {
    let swap = Swap::new(
        "IRS-USD-5Y",
        "JPM",
        usd(),
        Settlement::otc(),
        FixedLeg {
            notional: 10_000_000.0,
            rate: 0.035,
            day_count: DayCount::Thirty360,
            frequency: Frequency::SemiAnnual,
            start_date: Date::new(2025, 1, 15),
            end_date: Date::new(2030, 1, 15),
            direction: PayReceive::Pay,
        },
        FloatingLeg {
            notional: 10_000_000.0,
            rate_index_id: "USD-SOFR-3M".to_string(),
            spread: 0.001,
            day_count: DayCount::Act360,
            frequency: Frequency::Quarterly,
            start_date: Date::new(2025, 1, 15),
            end_date: Date::new(2030, 1, 15),
            direction: PayReceive::Receive,
        },
    );

    let inst: Arc<dyn FinancialInstrument> = Arc::new(swap.clone());
    let json = serde_json::to_string_pretty(&inst).unwrap();
    assert!(json.contains("\"type\": \"Swap\""));
    assert!(json.contains("\"USD-SOFR-3M\""));

    let deserialized: Arc<dyn FinancialInstrument> = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.id(), "IRS-USD-5Y");
    assert_eq!(deserialized.maturity(), Some(Date::new(2030, 1, 15)));

    assert_eq!(swap.effective_date(), Date::new(2025, 1, 15));
    assert_eq!(swap.termination_date(), Date::new(2030, 1, 15));
}

// ── FxForward ────────────────────────────────────────────────────────

#[test]
fn fx_forward_serde_roundtrip() {
    let fx = FxForward::new(
        "GBPUSD-6M",
        "BARCLAYS",
        gbp(),
        usd(),
        Settlement::otc(),
        1_000_000.0,
        -1_260_000.0,
        Date::new(2025, 12, 15),
    );

    let inst: Arc<dyn FinancialInstrument> = Arc::new(fx.clone());
    let json = serde_json::to_string_pretty(&inst).unwrap();
    assert!(json.contains("\"type\": \"FxForward\""));

    let deserialized: Arc<dyn FinancialInstrument> = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.id(), "GBPUSD-6M");

    assert!((fx.forward_rate() - (-1.26)).abs() < 1e-10);
}

// ── Basket ───────────────────────────────────────────────────────────

#[test]
fn basket_serde_roundtrip() {
    let equity = Arc::new(Equity::new("AAPL", "APPLE", usd(), Settlement::otc()));
    let option = Arc::new(EuropeanOption::new(
        "AAPL-C-190", "AAPL", "OCC", usd(), Settlement::otc(),
        Date::new(2025, 6, 20), 190.0, PutOrCall::Call,
        OptionSettlement::Cash, Date::new(2025, 6, 22),
    ));

    let basket = Basket::new(
        "BASKET-1",
        "HEDGE_FUND",
        usd(),
        Settlement::otc(),
        vec![
            (0.7, equity as Arc<dyn FinancialInstrument>),
            (0.3, option as Arc<dyn FinancialInstrument>),
        ],
    );

    assert_eq!(basket.len(), 2);
    assert!(!basket.is_empty());

    let inst: Arc<dyn FinancialInstrument> = Arc::new(basket);
    let json = serde_json::to_string_pretty(&inst).unwrap();
    assert!(json.contains("\"type\": \"Basket\""));
    assert!(json.contains("\"type\": \"Equity\""));
    assert!(json.contains("\"type\": \"EuropeanOption\""));

    let deserialized: Arc<dyn FinancialInstrument> = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.id(), "BASKET-1");
    // Maturity = max of components = option expiry
    assert_eq!(deserialized.maturity(), Some(Date::new(2025, 6, 20)));
}

// ── Nested basket ────────────────────────────────────────────────────

#[test]
fn nested_basket_serde_roundtrip() {
    let eq1 = Arc::new(Equity::new("AAPL", "APPLE", usd(), Settlement::otc()))
        as Arc<dyn FinancialInstrument>;
    let eq2 = Arc::new(Equity::new("MSFT", "MSFT_INC", usd(), Settlement::otc()))
        as Arc<dyn FinancialInstrument>;

    let inner_basket = Arc::new(Basket::new(
        "INNER", "FUND", usd(), Settlement::otc(),
        vec![(0.5, eq1), (0.5, eq2)],
    )) as Arc<dyn FinancialInstrument>;

    let future = Arc::new(Future::new(
        "ES-Jun25", "SP500", usd(), Settlement::otc(),
        Date::new(2025, 6, 20), 50.0, 0.25,
    )) as Arc<dyn FinancialInstrument>;

    let outer_basket = Basket::new(
        "OUTER", "FUND", usd(), Settlement::otc(),
        vec![(0.8, inner_basket), (0.2, future)],
    );

    let inst: Arc<dyn FinancialInstrument> = Arc::new(outer_basket);
    let json = serde_json::to_string_pretty(&inst).unwrap();

    let deserialized: Arc<dyn FinancialInstrument> = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.id(), "OUTER");
    assert_eq!(deserialized.maturity(), Some(Date::new(2025, 6, 20)));
}

// ── Deserialize from hand-written JSON ───────────────────────────────

#[test]
fn deserialize_future_from_json() {
    let json = r#"
    {
        "type": "Future",
        "id": "ICE-BRN-Jul25",
        "underlying": "Brent",
        "currency": {
            "id": "USD",
            "settlement": "Null",
            "day_count": "Act360"
        },
        "settlement": {
            "venue": "ICE",
            "session": "SETTLE",
            "time": "19:30",
            "timezone": "Europe/London"
        },
        "expiry": "2025-07-14",
        "contract_size": 1000.0,
        "tick_size": 0.01
    }
    "#;

    let inst: Arc<dyn FinancialInstrument> = serde_json::from_str(json).unwrap();
    assert_eq!(inst.id(), "ICE-BRN-Jul25");
    assert_eq!(inst.instrument_type(), "Future");
    assert_eq!(inst.currency().id, "USD");
    assert_eq!(inst.maturity(), Some(Date::new(2025, 7, 14)));
}

#[test]
fn malformed_json_gives_error() {
    let json = r#"{"type": "Future", "id": 42}"#;
    let result: Result<Arc<dyn FinancialInstrument>, _> = serde_json::from_str(json);
    assert!(result.is_err());
}

#[test]
fn unknown_type_gives_error() {
    let json = r#"{"type": "CreditDefaultSwap", "id": "nope"}"#;
    let result: Result<Arc<dyn FinancialInstrument>, _> = serde_json::from_str(json);
    assert!(result.is_err());
}

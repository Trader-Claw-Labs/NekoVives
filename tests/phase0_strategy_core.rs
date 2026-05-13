//! Phase 0 regression tests — verify that:
//! 1. `RunnerConfig` deserialises correctly with and without `kind` field.
//! 2. `kind = None` is treated as "rhai_candle" (legacy default).
//! 3. `strategy_core::engines` constants are stable.
//! 4. `strategy_core` types round-trip through JSON.
//!
//! These tests do NOT start a real runner or call Binance/Polymarket APIs.

use strategy_core::{
    engines,
    types::{ExecutionMode, OrderIntent, Portfolio, Side},
};
use trader_claw::strategy_runner::RunnerConfig;

// ── 1. RunnerConfig backward-compat ──────────────────────────────────────────

#[test]
fn runner_config_without_kind_deserialises() {
    let json = r#"{
        "id": "test-1",
        "name": "Test",
        "script": "polymarket_5min.rhai",
        "market_type": "polymarket_binary",
        "symbol": "BTCUSDT",
        "interval": "5m",
        "mode": "paper",
        "initial_balance": 1000.0,
        "fee_pct": 0.1,
        "warmup_days": 7
    }"#;

    let cfg: RunnerConfig = serde_json::from_str(json).expect("must deserialise");
    assert_eq!(cfg.id, "test-1");
    assert!(cfg.kind.is_none(), "kind should be None when omitted");
}

#[test]
fn runner_config_with_kind_deserialises() {
    let json = r#"{
        "id": "test-2",
        "name": "Arb",
        "script": "",
        "market_type": "polymarket_binary",
        "symbol": "",
        "interval": "1m",
        "mode": "paper",
        "initial_balance": 1000.0,
        "fee_pct": 0.1,
        "warmup_days": 1,
        "kind": "arb_binary"
    }"#;

    let cfg: RunnerConfig = serde_json::from_str(json).expect("must deserialise");
    assert_eq!(cfg.kind.as_deref(), Some("arb_binary"));
}

#[test]
fn runner_config_kind_round_trips() {
    let json_in = r#"{
        "id": "rt-1",
        "name": "RT",
        "script": "strategy.rhai",
        "market_type": "crypto",
        "symbol": "BTCUSDT",
        "interval": "4m",
        "mode": "paper",
        "initial_balance": 500.0,
        "fee_pct": 0.05,
        "warmup_days": 3,
        "kind": "minting_mm"
    }"#;

    let cfg: RunnerConfig = serde_json::from_str(json_in).unwrap();
    let json_out = serde_json::to_string(&cfg).unwrap();
    let cfg2: RunnerConfig = serde_json::from_str(&json_out).unwrap();
    assert_eq!(cfg2.kind.as_deref(), Some("minting_mm"));
}

// ── 2. Engine kind constants ──────────────────────────────────────────────────

#[test]
fn engine_kind_constants_stable() {
    assert_eq!(engines::RHAI_CANDLE, "rhai_candle");
    assert_eq!(engines::ARB_BINARY, "arb_binary");
    assert_eq!(engines::MINTING_MM, "minting_mm");
    assert_eq!(engines::ROTATION_COMPOUNDER, "rotation_compounder");
    assert_eq!(engines::FAIR_VALUE, "fair_value");
    assert_eq!(engines::FV_MOMENTUM, "fv_momentum");
}

#[test]
fn engine_kind_default_is_rhai_candle() {
    assert_eq!(engines::default_kind(), "rhai_candle");
}

#[test]
fn engine_kind_is_known() {
    for k in &["rhai_candle", "arb_binary", "minting_mm",
               "rotation_compounder", "fair_value", "fv_momentum"] {
        assert!(engines::is_known(k), "{k} should be known");
    }
    assert!(!engines::is_known("unknown_engine"));
    assert!(!engines::is_known(""));
}

// ── 3. Core types round-trip ──────────────────────────────────────────────────

#[test]
fn execution_mode_round_trips() {
    for mode in &[ExecutionMode::Backtest, ExecutionMode::DryRun, ExecutionMode::Live] {
        let s = serde_json::to_string(mode).unwrap();
        let back: ExecutionMode = serde_json::from_str(&s).unwrap();
        assert_eq!(&back, mode);
    }
}

#[test]
fn execution_mode_is_live() {
    assert!(!ExecutionMode::Backtest.is_live());
    assert!(!ExecutionMode::DryRun.is_live());
    assert!(ExecutionMode::Live.is_live());
}

#[test]
fn execution_mode_places_real_orders() {
    assert!(!ExecutionMode::Backtest.places_real_orders());
    assert!(!ExecutionMode::DryRun.places_real_orders());
    assert!(ExecutionMode::Live.places_real_orders());
}

#[test]
fn order_intent_hold_round_trips() {
    let intent = OrderIntent::Hold;
    let s = serde_json::to_string(&intent).unwrap();
    let back: OrderIntent = serde_json::from_str(&s).unwrap();
    assert!(matches!(back, OrderIntent::Hold));
}

#[test]
fn order_intent_buy_round_trips() {
    let intent = OrderIntent::Buy {
        token_id: "tok-yes-123".to_string(),
        side: Side::Yes,
        size_usd: 500.0,
        limit_price: Some(0.58),
    };
    let s = serde_json::to_string(&intent).unwrap();
    let back: OrderIntent = serde_json::from_str(&s).unwrap();
    match back {
        OrderIntent::Buy { token_id, size_usd, limit_price, .. } => {
            assert_eq!(token_id, "tok-yes-123");
            assert!((size_usd - 500.0).abs() < 1e-9);
            assert_eq!(limit_price, Some(0.58));
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn portfolio_total_return_pct_zero_initial() {
    let p = Portfolio::new(0.0);
    assert_eq!(p.total_return_pct(), 0.0);
}

#[test]
fn portfolio_total_return_pct_positive() {
    let mut p = Portfolio::new(1000.0);
    p.balance_usdc = 1100.0;
    let ret = p.total_return_pct();
    assert!((ret - 10.0).abs() < 1e-9, "expected 10%, got {ret}");
}

// ── 4. BookSnapshot helpers ───────────────────────────────────────────────────

#[test]
fn book_snapshot_arb_edge() {
    use strategy_core::types::{BookLevel, BookSnapshot};
    use chrono::Utc;

    let snap = BookSnapshot {
        market_id: "mkt-1".to_string(),
        slug: "test-slug".to_string(),
        yes: BookLevel { best_ask: 0.56, best_bid: 0.54, ask_depth_usd: 1000.0, bid_depth_usd: 800.0 },
        no: BookLevel { best_ask: 0.41, best_bid: 0.39, ask_depth_usd: 900.0, bid_depth_usd: 700.0 },
        timestamp: Utc::now(),
        meta: Default::default(),
    };

    // ask_sum = 0.56 + 0.41 = 0.97 → edge = 1.0 - 0.97 - fee
    let edge = snap.arb_edge(0.005);
    assert!((edge - 0.025).abs() < 1e-9, "expected 0.025, got {edge}");

    // ask_sum
    assert!((snap.ask_sum() - 0.97).abs() < 1e-9);
}

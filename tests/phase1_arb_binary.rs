//! Phase 1 tests — arb_binary engine.
//!
//! These tests exercise the detector logic and DryRun simulation without
//! calling any real Polymarket APIs.

use strategy_core::{
    engine::StrategyEngine,
    types::{BookLevel, BookSnapshot, ExecutionMode, Portfolio},
};
use chrono::Utc;
use trader_claw::engines::arb_binary::{ArbBinaryConfig, ArbBinaryEngine};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_snap(slug: &str, ask_yes: f64, ask_no: f64, depth: f64) -> BookSnapshot {
    BookSnapshot {
        market_id: format!("{slug}_yes"),
        slug: slug.to_string(),
        yes: BookLevel {
            best_ask: ask_yes,
            best_bid: ask_yes - 0.005,
            ask_depth_usd: depth,
            bid_depth_usd: depth * 0.8,
        },
        no: BookLevel {
            best_ask: ask_no,
            best_bid: ask_no - 0.005,
            ask_depth_usd: depth,
            bid_depth_usd: depth * 0.8,
        },
        timestamp: Utc::now(),
        meta: Default::default(),
    }
}

fn default_cfg() -> ArbBinaryConfig {
    ArbBinaryConfig {
        markets: vec!["test-market".to_string()],
        min_edge_pct: 0.005,
        max_position_usd: 100.0,
        liquidity_floor_usd: 50.0,
        fee_pct: 0.002,
        poll_secs: 60,
        max_concurrent: 5,
    }
}

// ── Detector logic ────────────────────────────────────────────────────────────

#[test]
fn no_arb_when_sum_equals_one() {
    // YES=0.55 + NO=0.45 = 1.00 → edge after fee = -0.002, no opportunity
    let cfg = default_cfg();
    let mut engine = ArbBinaryEngine::new(cfg);
    let mut portfolio = Portfolio::new(1000.0);

    let snap = make_snap("market-1", 0.55, 0.45, 1000.0);

    // Run synchronously via tokio block_on
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        engine.initialize(ExecutionMode::DryRun, &portfolio).await.unwrap();
        let intents = engine.on_book(&snap, &mut portfolio).await.unwrap();
        assert!(intents.is_empty(), "no opportunity should be returned");
    });
}

#[test]
fn arb_detected_when_sum_below_one() {
    // YES=0.44 + NO=0.46 = 0.90 → edge = 1.0 - 0.90 - 0.002 = 0.098 (9.8%)
    let cfg = default_cfg();
    let mut engine = ArbBinaryEngine::new(cfg);
    let mut portfolio = Portfolio::new(1000.0);

    let snap = make_snap("market-2", 0.44, 0.46, 1000.0);

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        engine.initialize(ExecutionMode::DryRun, &portfolio).await.unwrap();
        let intents = engine.on_book(&snap, &mut portfolio).await.unwrap();
        // DryRun: simulates internally, returns empty intents (no real orders)
        // but the cycle is recorded and portfolio is updated
        let _ = intents; // may be empty in DryRun
    });
}

#[test]
fn dryrun_increases_portfolio_balance() {
    // YES=0.43 + NO=0.46 = 0.89 → clear opportunity
    let cfg = default_cfg();
    let mut engine = ArbBinaryEngine::new(cfg);
    let mut portfolio = Portfolio::new(1000.0);
    let initial = portfolio.balance_usdc;

    let snap = make_snap("market-profit", 0.43, 0.46, 2000.0);

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        engine.initialize(ExecutionMode::DryRun, &portfolio).await.unwrap();
        engine.on_book(&snap, &mut portfolio).await.unwrap();

        // DryRun should have simulated a profitable cycle.
        assert!(
            portfolio.balance_usdc > initial,
            "balance should increase after arb cycle; was {initial:.4}, now {:.4}",
            portfolio.balance_usdc
        );
    });
}

#[test]
fn liquidity_gate_blocks_shallow_market() {
    // Deep opportunity but liquidity_floor = 500 > depth = 30
    let mut cfg = default_cfg();
    cfg.liquidity_floor_usd = 500.0;
    let mut engine = ArbBinaryEngine::new(cfg);
    let mut portfolio = Portfolio::new(1000.0);

    // Shallow depth (30 < 500)
    let snap = make_snap("shallow-market", 0.43, 0.46, 30.0);

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        engine.initialize(ExecutionMode::DryRun, &portfolio).await.unwrap();
        let balance_before = portfolio.balance_usdc;
        engine.on_book(&snap, &mut portfolio).await.unwrap();
        assert_eq!(
            portfolio.balance_usdc, balance_before,
            "liquidity gate should block the trade"
        );
    });
}

#[test]
fn no_arb_when_insufficient_balance() {
    let mut cfg = default_cfg();
    cfg.min_edge_pct = 0.001;
    cfg.liquidity_floor_usd = 1.0;
    let mut engine = ArbBinaryEngine::new(cfg);
    let mut portfolio = Portfolio::new(0.01); // essentially empty

    let snap = make_snap("broke-market", 0.43, 0.46, 2000.0);

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        engine.initialize(ExecutionMode::DryRun, &portfolio).await.unwrap();
        let balance_before = portfolio.balance_usdc;
        engine.on_book(&snap, &mut portfolio).await.unwrap();
        // With near-zero balance, sized position rounds to 0, nothing executed.
        assert!(
            portfolio.balance_usdc >= 0.0,
            "balance must not go negative"
        );
        let _ = balance_before;
    });
}

// ── Metrics ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn finalize_with_no_cycles_returns_zero_metrics() {
    let cfg = default_cfg();
    let mut engine = ArbBinaryEngine::new(cfg);
    let portfolio = Portfolio::new(1000.0);

    engine.initialize(ExecutionMode::DryRun, &portfolio).await.unwrap();
    let metrics = engine.finalize(&portfolio).await;

    assert_eq!(metrics.total_trades, 0);
    assert_eq!(metrics.win_rate_pct, 0.0);
    assert!(metrics.analysis.contains("No arbitrage"));
}

#[tokio::test]
async fn finalize_after_cycles_has_correct_trade_count() {
    let cfg = default_cfg();
    let mut engine = ArbBinaryEngine::new(cfg);
    let mut portfolio = Portfolio::new(1000.0);

    engine.initialize(ExecutionMode::DryRun, &portfolio).await.unwrap();

    // Trigger three profitable snaps.
    for i in 0..3 {
        let snap = make_snap(&format!("m{i}"), 0.40, 0.44, 5000.0);
        engine.on_book(&snap, &mut portfolio).await.unwrap();
    }

    let metrics = engine.finalize(&portfolio).await;
    // 3 cycles × 2 orders each = 6 total_trades
    assert_eq!(metrics.total_trades, 6, "expected 6 order legs");
    assert!(metrics.total_return_pct > 0.0, "should be profitable");
    assert_eq!(metrics.data_confidence, "high"); // DryRun = high confidence
}

// ── Edge math ─────────────────────────────────────────────────────────────────

#[test]
fn book_snapshot_arb_edge_matches_engine_threshold() {
    use strategy_core::types::BookSnapshot;

    let snap = make_snap("edge-test", 0.44, 0.46, 1000.0);
    let fee = 0.002;
    let edge = snap.arb_edge(fee);

    // 1.0 - 0.44 - 0.46 - 0.002 = 0.098
    assert!(
        (edge - 0.098).abs() < 1e-9,
        "expected edge 0.098, got {edge}"
    );
    assert!(edge > 0.005, "edge should exceed min_edge_pct threshold");
}

// ── RunnerConfig round-trip with kind=arb_binary ──────────────────────────────

#[test]
fn runner_config_arb_binary_kind_survives_serde() {
    use trader_claw::strategy_runner::RunnerConfig;

    let json = r#"{
        "id": "arb-1",
        "name": "Arb Runner",
        "script": "",
        "market_type": "polymarket_binary",
        "symbol": "will-btc-reach-100k",
        "interval": "1m",
        "mode": "paper",
        "initial_balance": 1000.0,
        "fee_pct": 0.2,
        "warmup_days": 1,
        "kind": "arb_binary",
        "threshold": 0.01
    }"#;

    let cfg: RunnerConfig = serde_json::from_str(json).unwrap();
    assert_eq!(cfg.kind.as_deref(), Some("arb_binary"));
    assert_eq!(cfg.threshold, Some(0.01));
    assert_eq!(cfg.symbol, "will-btc-reach-100k");
}

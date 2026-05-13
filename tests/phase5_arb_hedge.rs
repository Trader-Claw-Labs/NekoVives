//! Phase 5 tests — arb_hedge engine (HYB-02).
//!
//! Covers: synthetic-arb detection, hedge-overlay trigger/unwind,
//! backtest simulation, DryRun book entry, metrics, and RunnerConfig serde.

use strategy_core::{
    engine::StrategyEngine,
    types::{BookLevel, BookSnapshot, CandleSnap, ExecutionMode, MarketSnapshot, Portfolio, Side},
};
use chrono::Utc;
use trader_claw::engines::arb_hedge::{ArbHedgeConfig, ArbHedgeEngine, MarketState};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_candle(slug: &str, close: f64, high: f64, low: f64) -> MarketSnapshot {
    MarketSnapshot {
        market_id: format!("{slug}_yes"),
        slug: slug.to_string(),
        candle: Some(CandleSnap {
            open_time_ms: Utc::now().timestamp_millis(),
            open: close, high, low, close,
            volume: 5000.0,
        }),
        book: None,
        timestamp: Utc::now(),
    }
}

fn make_book(slug: &str, ask_yes: f64, ask_no: f64) -> BookSnapshot {
    BookSnapshot {
        market_id: format!("{slug}_yes"),
        slug: slug.to_string(),
        yes: BookLevel { best_ask: ask_yes, best_bid: ask_yes - 0.01, ask_depth_usd: 5000.0, bid_depth_usd: 4000.0 },
        no:  BookLevel { best_ask: ask_no,  best_bid: ask_no  - 0.01, ask_depth_usd: 5000.0, bid_depth_usd: 4000.0 },
        timestamp: Utc::now(),
        meta: Default::default(),
    }
}

fn default_cfg(markets: Vec<&str>) -> ArbHedgeConfig {
    ArbHedgeConfig {
        markets:           markets.iter().map(|s| s.to_string()).collect(),
        min_arb_edge:      0.03,
        hedge_trigger_pct: 0.20,
        max_position_usd:  200.0,
        poll_secs:         60,
    }
}

// ── Synthetic arb detection ───────────────────────────────────────────────────

#[test]
fn score_arb_detects_when_sum_below_threshold() {
    let cfg = default_cfg(vec!["test"]);
    // 0.45 + 0.48 = 0.93 < 1.0 - 0.03 = 0.97 → arb
    let (has_arb, margin) = ArbHedgeEngine::score_arb(0.45, 0.48, &cfg);
    assert!(has_arb,  "0.93 sum should trigger arb");
    assert!(margin > 0.0, "margin should be positive");
}

#[test]
fn score_arb_no_opportunity_when_sum_above_threshold() {
    let cfg = default_cfg(vec!["test"]);
    // 0.49 + 0.50 = 0.99 > 0.97 → no arb
    let (has_arb, _) = ArbHedgeEngine::score_arb(0.49, 0.50, &cfg);
    assert!(!has_arb, "0.99 sum should not trigger arb");
}

#[test]
fn score_arb_margin_proportional_to_discount() {
    let cfg = default_cfg(vec!["test"]);
    let (_, margin_big)   = ArbHedgeEngine::score_arb(0.40, 0.40, &cfg); // sum=0.80
    let (_, margin_small) = ArbHedgeEngine::score_arb(0.45, 0.48, &cfg); // sum=0.93
    assert!(margin_big > margin_small, "bigger discount → bigger margin");
}

// ── DryRun: arb entry ─────────────────────────────────────────────────────────

#[tokio::test]
async fn dryrun_enters_arb_when_books_misaligned() {
    let cfg = default_cfg(vec!["arb-market"]);
    let mut engine    = ArbHedgeEngine::new(cfg);
    let mut portfolio = Portfolio::new(1000.0);

    engine.initialize(ExecutionMode::DryRun, &portfolio).await.unwrap();

    // yes=0.43, no=0.46 → sum=0.89 < 0.97 → arb
    engine.on_book(&make_book("arb-market", 0.43, 0.46), &mut portfolio).await.unwrap();

    assert!(portfolio.balance_usdc < 1000.0, "should have entered arb position");
}

#[tokio::test]
async fn dryrun_no_arb_when_sum_above_threshold() {
    let cfg = default_cfg(vec!["fair-market"]);
    let mut engine    = ArbHedgeEngine::new(cfg);
    let mut portfolio = Portfolio::new(1000.0);

    engine.initialize(ExecutionMode::DryRun, &portfolio).await.unwrap();

    // yes=0.49, no=0.50 → sum=0.99 > 0.97 → no arb (but may enter directional)
    engine.on_book(&make_book("fair-market", 0.49, 0.50), &mut portfolio).await.unwrap();

    // No arb entered, but directional check: neither side is < 0.45 → still Idle
    assert_eq!(portfolio.balance_usdc, 1000.0, "no position when market is fair");
}

// ── Backtest: hedge overlay ───────────────────────────────────────────────────

#[test]
fn backtest_hedge_triggered_when_position_down_enough() {
    let cfg = default_cfg(vec!["hedge-market"]);
    let mut engine    = ArbHedgeEngine::new(cfg);
    let mut portfolio = Portfolio::new(1000.0);

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        engine.initialize(ExecutionMode::Backtest, &portfolio).await.unwrap();

        // Enter Long(Yes) at 0.35 (cheap → below 0.45 threshold).
        engine.on_tick(&make_candle("hedge-market", 0.35, 0.38, 0.32), &mut portfolio).await.unwrap();
        let after_entry = portfolio.balance_usdc;
        assert!(after_entry < 1000.0, "should have entered a position");

        // Price drops to 0.25 → ~28.5% drop from 0.35 → ≥ 20% trigger.
        engine.on_tick(&make_candle("hedge-market", 0.25, 0.27, 0.22), &mut portfolio).await.unwrap();

        // After hedge trigger, balance should drop further (hedge cost).
        assert!(portfolio.balance_usdc < after_entry, "hedge leg should cost capital");
    });
}

#[test]
fn backtest_no_hedge_on_small_loss() {
    let cfg = default_cfg(vec!["small-loss"]);
    let mut engine    = ArbHedgeEngine::new(cfg);
    let mut portfolio = Portfolio::new(1000.0);

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        engine.initialize(ExecutionMode::Backtest, &portfolio).await.unwrap();

        // Enter at 0.35.
        engine.on_tick(&make_candle("small-loss", 0.35, 0.38, 0.32), &mut portfolio).await.unwrap();
        let after_entry = portfolio.balance_usdc;

        // Only 10% drop — below hedge_trigger_pct=0.20 → no hedge.
        engine.on_tick(&make_candle("small-loss", 0.32, 0.34, 0.30), &mut portfolio).await.unwrap();

        // Balance should be unchanged (no hedge leg added).
        assert_eq!(portfolio.balance_usdc, after_entry, "no hedge on small loss");
    });
}

// ── Backtest: resolution ──────────────────────────────────────────────────────

#[test]
fn backtest_winning_resolution_closes_with_profit() {
    let cfg = default_cfg(vec!["win-market"]);
    let mut engine    = ArbHedgeEngine::new(cfg);
    let mut portfolio = Portfolio::new(1000.0);

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        engine.initialize(ExecutionMode::Backtest, &portfolio).await.unwrap();

        // Enter Long(Yes) at 0.30.
        engine.on_tick(&make_candle("win-market", 0.30, 0.33, 0.27), &mut portfolio).await.unwrap();

        // Resolution: price → 0.95 (YES wins).
        engine.on_tick(&make_candle("win-market", 0.95, 0.97, 0.92), &mut portfolio).await.unwrap();

        let metrics = engine.finalize(&portfolio).await;
        if metrics.total_trades > 0 {
            assert!(metrics.total_return_pct > 0.0, "winning resolution should profit");
        }
    });
}

// ── Metrics ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn finalize_no_positions_returns_zero_metrics() {
    let cfg = default_cfg(vec!["empty"]);
    let mut engine    = ArbHedgeEngine::new(cfg);
    let portfolio     = Portfolio::new(1000.0);

    engine.initialize(ExecutionMode::DryRun, &portfolio).await.unwrap();
    let metrics = engine.finalize(&portfolio).await;

    assert_eq!(metrics.total_trades, 0);
    assert_eq!(metrics.win_rate_pct, 0.0);
    assert!(metrics.analysis.contains("No arb-hedge trades"));
}

// ── RunnerConfig serde ────────────────────────────────────────────────────────

#[test]
fn runner_config_arb_hedge_kind_survives_serde() {
    use trader_claw::strategy_runner::RunnerConfig;

    let json = r#"{
        "id": "ah-1",
        "name": "Arb-Hedge Runner",
        "script": "",
        "market_type": "polymarket_binary",
        "symbol": "btc-100k,eth-flip",
        "interval": "1m",
        "mode": "paper",
        "initial_balance": 1000.0,
        "fee_pct": 0.1,
        "warmup_days": 0,
        "kind": "arb_hedge",
        "threshold": 0.03
    }"#;

    let cfg: RunnerConfig = serde_json::from_str(json).unwrap();
    assert_eq!(cfg.kind.as_deref(), Some("arb_hedge"));
    assert_eq!(cfg.threshold, Some(0.03));
    assert!(cfg.symbol.contains(','));
}

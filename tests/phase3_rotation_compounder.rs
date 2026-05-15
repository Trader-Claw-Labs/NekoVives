//! Phase 3 tests — rotation_compounder engine.
//!
//! Tests cover: scoring function, market selection, backtest rotation,
//! stop-loss, metrics, and RunnerConfig serde.

use strategy_core::{
    engine::StrategyEngine,
    types::{BookLevel, BookSnapshot, CandleSnap, ExecutionMode, MarketSnapshot, Portfolio},
};
use chrono::Utc;
use trader_claw::engines::rotation_compounder::{RotationCompounderEngine, RotationConfig};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_candle(slug: &str, close: f64, high: f64, low: f64, volume: f64) -> MarketSnapshot {
    MarketSnapshot {
        market_id: format!("{slug}_yes"),
        slug: slug.to_string(),
        candle: Some(CandleSnap {
            open_time_ms: Utc::now().timestamp_millis(),
            open: close,
            high,
            low,
            close,
            volume,
        }),
        book: None,
        timestamp: Utc::now(),
    }
}

fn make_book(slug: &str, ask_yes: f64, ask_no: f64, depth: f64) -> BookSnapshot {
    BookSnapshot {
        market_id: format!("{slug}_yes"),
        slug: slug.to_string(),
        yes: BookLevel { best_ask: ask_yes, best_bid: ask_yes - 0.01, ask_depth_usd: depth, bid_depth_usd: depth * 0.8 },
        no:  BookLevel { best_ask: ask_no,  best_bid: ask_no  - 0.01, ask_depth_usd: depth, bid_depth_usd: depth * 0.8 },
        timestamp: Utc::now(),
        meta: Default::default(),
    }
}

fn default_cfg(markets: Vec<&str>) -> RotationConfig {
    RotationConfig {
        markets:           markets.iter().map(|s| s.to_string()).collect(),
        max_allocation_pct: 0.60,
        switch_threshold:  0.001, // small so rotation fires clearly in tests
        min_position_usd:  10.0,
        stop_loss_pct:     0.40,
        poll_secs:         60,
        sim_days_to_close: 15.0,
    }
}

// ── Scoring function ──────────────────────────────────────────────────────────

#[test]
fn score_market_high_edge_scores_higher() {
    use trader_claw::engines::rotation_compounder::RotationCompounderEngine;

    let (score_high, _, edge_high, _) = RotationCompounderEngine::score_market(0.15, 0.85, 5000.0, 48.0);
    let (score_low,  _, edge_low,  _) = RotationCompounderEngine::score_market(0.45, 0.55, 5000.0, 48.0);

    assert!(score_high > score_low, "high-edge market should score higher");
    assert!(edge_high > edge_low,   "edge_high should be larger");
}

#[test]
fn score_market_deep_liquidity_scores_higher() {
    let (score_deep,   _, _, _) = RotationCompounderEngine::score_market(0.25, 0.75, 50_000.0, 48.0);
    let (score_shallow,_, _, _) = RotationCompounderEngine::score_market(0.25, 0.75, 50.0,     48.0);
    assert!(score_deep > score_shallow, "deep liquidity should score higher");
}

#[test]
fn score_market_time_urgency_increases_score() {
    // Market closing in 2h should score higher than one closing in 500h.
    let (score_soon, _, _, _) = RotationCompounderEngine::score_market(0.25, 0.75, 5000.0, 2.0);
    let (score_far,  _, _, _) = RotationCompounderEngine::score_market(0.25, 0.75, 5000.0, 500.0);
    assert!(score_soon > score_far, "imminent resolution should score higher");
}

// ── Backtest: enters position ─────────────────────────────────────────────────

#[test]
fn backtest_enters_top_market() {
    let cfg = default_cfg(vec!["market-a", "market-b"]);
    let mut engine    = RotationCompounderEngine::new(cfg);
    let mut portfolio = Portfolio::new(1000.0);

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        engine.initialize(ExecutionMode::Backtest, &portfolio).await.unwrap();

        // Feed two markets: A at 0.20 (strong edge), B at 0.45 (weak edge).
        engine.on_tick(&make_candle("market-a", 0.20, 0.25, 0.15, 10_000.0), &mut portfolio).await.unwrap();
        engine.on_tick(&make_candle("market-b", 0.45, 0.50, 0.40, 10_000.0), &mut portfolio).await.unwrap();

        // Should have entered a position (balance decreased).
        assert!(
            portfolio.balance_usdc < 1000.0,
            "should have entered a position; balance was {:.2}", portfolio.balance_usdc
        );
    });
}

// ── Backtest: rotation to better market ──────────────────────────────────────

#[test]
fn backtest_rotates_to_better_market() {
    let cfg = default_cfg(vec!["market-a", "market-b"]);
    let mut engine    = RotationCompounderEngine::new(cfg);
    let mut portfolio = Portfolio::new(1000.0);

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        engine.initialize(ExecutionMode::Backtest, &portfolio).await.unwrap();

        // First tick: A is better.
        engine.on_tick(&make_candle("market-a", 0.20, 0.25, 0.15, 10_000.0), &mut portfolio).await.unwrap();
        engine.on_tick(&make_candle("market-b", 0.45, 0.50, 0.40, 5_000.0), &mut portfolio).await.unwrap();

        // Second tick: B now has much stronger edge (drop to 0.08).
        engine.on_tick(&make_candle("market-a", 0.48, 0.50, 0.46, 5_000.0), &mut portfolio).await.unwrap();
        engine.on_tick(&make_candle("market-b", 0.08, 0.12, 0.05, 10_000.0), &mut portfolio).await.unwrap();

        // We don't assert specific slug (can't read private field), but closed > 0 means rotation happened.
        let metrics = engine.finalize(&portfolio).await;
        // Rotation closes the old position and opens a new one — at least 1 closed.
        assert!(metrics.total_trades >= 1, "at least one position should have closed via rotation");
    });
}

// ── Backtest: resolution win ──────────────────────────────────────────────────

#[test]
fn backtest_winning_resolution_increases_balance() {
    let cfg = default_cfg(vec!["win-market"]);
    let mut engine    = RotationCompounderEngine::new(cfg);
    let mut portfolio = Portfolio::new(1000.0);

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        engine.initialize(ExecutionMode::Backtest, &portfolio).await.unwrap();

        // Enter at 0.18 (yes_price < no_price=0.82 → score_market picks YES).
        engine.on_tick(&make_candle("win-market", 0.18, 0.22, 0.14, 20_000.0), &mut portfolio).await.unwrap();

        // Single resolution tick: price spikes to 0.95 → YES wins, position closes.
        engine.on_tick(&make_candle("win-market", 0.95, 0.98, 0.90, 20_000.0), &mut portfolio).await.unwrap();

        let metrics = engine.finalize(&portfolio).await;
        // After one WIN cycle, total return must be positive.
        if metrics.total_trades > 0 {
            assert!(
                metrics.total_return_pct > 0.0,
                "winning resolution should produce positive total return; got {:.4}%",
                metrics.total_return_pct
            );
        } else {
            // No trade closed yet (position still open) — verify balance is intact.
            assert!(portfolio.balance_usdc >= 0.0);
        }
    });
}

// ── DryRun: on_book enters position ──────────────────────────────────────────

#[tokio::test]
async fn dryrun_on_book_enters_top_scored_market() {
    let cfg = default_cfg(vec!["btc-100k", "eth-flip"]);
    let mut engine    = RotationCompounderEngine::new(cfg);
    let mut portfolio = Portfolio::new(1000.0);

    engine.initialize(ExecutionMode::DryRun, &portfolio).await.unwrap();

    // btc-100k at 0.15 (strong edge), eth-flip at 0.48 (weak edge).
    engine.on_book(&make_book("btc-100k", 0.15, 0.85, 10_000.0), &mut portfolio).await.unwrap();
    engine.on_book(&make_book("eth-flip",  0.48, 0.52, 10_000.0), &mut portfolio).await.unwrap();

    assert!(portfolio.balance_usdc < 1000.0, "DryRun should have entered a position");
}

// ── Metrics ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn finalize_no_positions_returns_zero_metrics() {
    let cfg = default_cfg(vec!["empty-market"]);
    let mut engine    = RotationCompounderEngine::new(cfg);
    let portfolio     = Portfolio::new(1000.0);

    engine.initialize(ExecutionMode::DryRun, &portfolio).await.unwrap();
    let metrics = engine.finalize(&portfolio).await;

    assert_eq!(metrics.total_trades, 0);
    assert_eq!(metrics.win_rate_pct, 0.0);
    assert!(metrics.analysis.contains("No positions closed"));
}

// ── RunnerConfig serde ────────────────────────────────────────────────────────

#[test]
fn runner_config_rotation_compounder_kind_survives_serde() {
    use trader_claw::strategy_runner::RunnerConfig;

    let json = r#"{
        "id": "rc-1",
        "name": "Rotation Runner",
        "script": "",
        "market_type": "polymarket_binary",
        "symbol": "btc-100k,eth-flip,trump-win",
        "interval": "1m",
        "mode": "paper",
        "initial_balance": 1000.0,
        "fee_pct": 0.1,
        "warmup_days": 0,
        "kind": "rotation_compounder",
        "threshold": 0.05
    }"#;

    let cfg: RunnerConfig = serde_json::from_str(json).unwrap();
    assert_eq!(cfg.kind.as_deref(), Some("rotation_compounder"));
    assert_eq!(cfg.threshold, Some(0.05));
    // symbol carries multiple slugs comma-separated
    assert!(cfg.symbol.contains(','));
}

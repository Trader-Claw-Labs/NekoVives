//! Phase 2 tests — minting_mm engine.
//!
//! Tests cover: FSM state transitions, DryRun simulation, Backtest simulation,
//! metrics correctness, and RunnerConfig round-trip with kind=minting_mm.

use strategy_core::{
    engine::StrategyEngine,
    types::{BookLevel, BookSnapshot, CandleSnap, ExecutionMode, MarketSnapshot, Portfolio},
};
use chrono::Utc;
use trader_claw::engines::minting_mm::{MintingMmConfig, MintingMmEngine, MmState};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_book(slug: &str, ask_yes: f64, ask_no: f64, bid_yes: f64, bid_no: f64) -> BookSnapshot {
    BookSnapshot {
        market_id: format!("{slug}_yes"),
        slug: slug.to_string(),
        yes: BookLevel {
            best_ask: ask_yes,
            best_bid: bid_yes,
            ask_depth_usd: 5000.0,
            bid_depth_usd: 4000.0,
        },
        no: BookLevel {
            best_ask: ask_no,
            best_bid: bid_no,
            ask_depth_usd: 5000.0,
            bid_depth_usd: 4000.0,
        },
        timestamp: Utc::now(),
        meta: Default::default(),
    }
}

fn make_candle(close: f64, high: f64, low: f64) -> MarketSnapshot {
    MarketSnapshot {
        market_id: "backtest-market_yes".to_string(),
        slug: "backtest-market".to_string(),
        candle: Some(CandleSnap {
            open_time_ms: Utc::now().timestamp_millis(),
            open: close,
            high,
            low,
            close,
            volume: 100_000.0,
        }),
        book: None,
        timestamp: Utc::now(),
    }
}

fn default_cfg() -> MintingMmConfig {
    MintingMmConfig {
        markets: vec!["test-market".to_string()],
        premium_cents: 0.02,
        max_cycle_usd: 100.0,
        cycle_hours: 24,
        target_apy: 0.40,
        min_spread: 0.04,
        poll_secs: 30,
        collateral: "usdc_e".to_string(),
    }
}

// ── FSM state transitions (DryRun) ───────────────────────────────────────────

#[tokio::test]
async fn dryrun_idle_to_minted_on_first_book() {
    let cfg = default_cfg();
    let mut engine = MintingMmEngine::new(cfg);
    let mut portfolio = Portfolio::new(1000.0);

    // Wide spread market (YES: 0.52-0.56, spread = 0.04)
    let snap = make_book("test-market", 0.56, 0.46, 0.52, 0.42);

    engine.initialize(ExecutionMode::DryRun, &portfolio).await.unwrap();
    engine.on_book(&snap, &mut portfolio).await.unwrap();

    // Should have transitioned Idle → Minted, debiting balance.
    let state = engine.state_of("test-market");
    assert!(
        matches!(state, MmState::Minted { .. }),
        "expected Minted state, got {state:?}"
    );
    assert!(
        portfolio.balance_usdc < 1000.0,
        "balance should have been debited for mint"
    );
}

#[tokio::test]
async fn dryrun_minted_to_orders_placed_on_second_book() {
    let cfg = default_cfg();
    let mut engine = MintingMmEngine::new(cfg);
    let mut portfolio = Portfolio::new(1000.0);
    let snap = make_book("test-market", 0.56, 0.46, 0.52, 0.42);

    engine.initialize(ExecutionMode::DryRun, &portfolio).await.unwrap();
    engine.on_book(&snap, &mut portfolio).await.unwrap(); // Idle → Minted
    engine.on_book(&snap, &mut portfolio).await.unwrap(); // Minted → OrdersPlaced

    let state = engine.state_of("test-market");
    assert!(
        matches!(state, MmState::OrdersPlaced { .. }),
        "expected OrdersPlaced, got {state:?}"
    );
}

#[tokio::test]
async fn dryrun_fills_when_price_rises_enough() {
    let cfg = default_cfg();
    let mut engine = MintingMmEngine::new(cfg);
    let mut portfolio = Portfolio::new(1000.0);
    let initial = portfolio.balance_usdc;

    let snap_wide = make_book("test-market", 0.56, 0.46, 0.52, 0.42);

    engine.initialize(ExecutionMode::DryRun, &portfolio).await.unwrap();
    engine.on_book(&snap_wide, &mut portfolio).await.unwrap(); // → Minted
    engine.on_book(&snap_wide, &mut portfolio).await.unwrap(); // → OrdersPlaced

    // Simulate the market ask rising past our sell price (fill condition).
    // sell_price_yes ≈ 0.56 + 0.02 = 0.58, sell_price_no ≈ 0.46 + 0.02 = 0.48
    // New ask_yes >= 0.575 and ask_no >= 0.475 triggers fill in DryRun.
    // Bid must be spread >= min_spread (0.04) from ask: bid = ask - 0.05.
    let snap_higher = make_book("test-market", 0.585, 0.485, 0.535, 0.435);
    engine.on_book(&snap_higher, &mut portfolio).await.unwrap(); // → Filled
    engine.on_book(&snap_higher, &mut portfolio).await.unwrap(); // Filled → Cycled/Idle + credit

    // After cycle completes the balance should have recovered (plus premium).
    assert!(
        portfolio.balance_usdc >= initial * 0.99,
        "balance should be close to or above initial after a fill cycle: was {initial:.2}, now {:.2}",
        portfolio.balance_usdc
    );
}

#[tokio::test]
async fn dryrun_narrow_spread_does_not_enter() {
    let cfg = default_cfg();
    let mut engine = MintingMmEngine::new(cfg);
    let mut portfolio = Portfolio::new(1000.0);
    let initial = portfolio.balance_usdc;

    // Narrow spread: ask_yes - bid_yes = 0.02 < min_spread 0.04
    let snap_narrow = make_book("test-market", 0.52, 0.48, 0.50, 0.46);

    engine.initialize(ExecutionMode::DryRun, &portfolio).await.unwrap();
    engine.on_book(&snap_narrow, &mut portfolio).await.unwrap();

    let state = engine.state_of("test-market");
    assert!(
        matches!(state, MmState::Idle),
        "narrow spread should keep state Idle, got {state:?}"
    );
    assert_eq!(portfolio.balance_usdc, initial, "no balance should be debited");
}

// ── Backtest simulation ───────────────────────────────────────────────────────

#[test]
fn backtest_wide_spread_produces_profit() {
    let cfg = default_cfg();
    let mut engine = MintingMmEngine::new(cfg);
    let mut portfolio = Portfolio::new(1000.0);
    let initial = portfolio.balance_usdc;

    // Candle with wide spread: high - low = 0.10 > 2 × 0.02 → BothFilled cycle.
    let snap = make_candle(0.55, 0.65, 0.45);

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        engine.initialize(ExecutionMode::Backtest, &portfolio).await.unwrap();
        engine.on_tick(&snap, &mut portfolio).await.unwrap();
    });

    assert!(
        portfolio.balance_usdc > initial,
        "backtest: wide spread should produce profit; was {initial:.4}, now {:.4}",
        portfolio.balance_usdc
    );
}

#[test]
fn backtest_narrow_spread_produces_small_loss() {
    let cfg = default_cfg();
    let mut engine = MintingMmEngine::new(cfg);
    let mut portfolio = Portfolio::new(1000.0);
    let initial = portfolio.balance_usdc;

    // Narrow candle: high - low = 0.01 < 2 × 0.02 → Timeout cycle (merge back at small loss).
    let snap = make_candle(0.50, 0.505, 0.495);

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        engine.initialize(ExecutionMode::Backtest, &portfolio).await.unwrap();
        engine.on_tick(&snap, &mut portfolio).await.unwrap();
    });

    assert!(
        portfolio.balance_usdc < initial,
        "backtest: narrow spread should produce small loss (merge fee); was {initial:.4}, now {:.4}",
        portfolio.balance_usdc
    );
    assert!(
        portfolio.balance_usdc > initial * 0.99,
        "merge loss should be < 1% of initial balance"
    );
}

// ── Metrics ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn finalize_with_no_cycles_returns_zero_metrics() {
    let cfg = default_cfg();
    let mut engine = MintingMmEngine::new(cfg);
    let portfolio = Portfolio::new(1000.0);

    engine.initialize(ExecutionMode::DryRun, &portfolio).await.unwrap();
    let metrics = engine.finalize(&portfolio).await;

    assert_eq!(metrics.total_trades, 0);
    assert_eq!(metrics.win_rate_pct, 0.0);
    assert!(metrics.analysis.contains("No minting cycles"));
}

#[test]
fn backtest_finalize_reflects_cycle_count() {
    let cfg = default_cfg();
    let mut engine = MintingMmEngine::new(cfg);
    let mut portfolio = Portfolio::new(2000.0);

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        engine.initialize(ExecutionMode::Backtest, &portfolio).await.unwrap();

        for i in 0..5 {
            // Alternate wide/narrow candles to get both BothFilled and Timeout cycles.
            let snap = if i % 2 == 0 {
                make_candle(0.55, 0.65, 0.45) // wide → BothFilled
            } else {
                make_candle(0.50, 0.505, 0.495) // narrow → Timeout
            };
            engine.on_tick(&snap, &mut portfolio).await.unwrap();
        }

        let metrics = engine.finalize(&portfolio).await;
        // 5 cycles × 2 order legs each
        assert_eq!(metrics.total_trades, 10, "expected 10 order legs (5 cycles × 2)");
        assert!(metrics.win_rate_pct > 0.0, "some cycles should be profitable");
        assert!(metrics.win_rate_pct < 100.0, "some cycles should time out");
        assert_eq!(metrics.data_confidence, "medium");
        assert!(metrics.analysis.contains("minting_mm:"));
    });
}

// ── RunnerConfig round-trip with kind=minting_mm ──────────────────────────────

#[test]
fn runner_config_minting_mm_kind_survives_serde() {
    use trader_claw::strategy_runner::RunnerConfig;

    let json = r#"{
        "id": "mm-1",
        "name": "Minting MM Runner",
        "script": "",
        "market_type": "polymarket_binary",
        "symbol": "will-eth-flip-btc",
        "interval": "1m",
        "mode": "paper",
        "initial_balance": 500.0,
        "fee_pct": 0.1,
        "warmup_days": 0,
        "kind": "minting_mm",
        "threshold": 0.025
    }"#;

    let cfg: RunnerConfig = serde_json::from_str(json).unwrap();
    assert_eq!(cfg.kind.as_deref(), Some("minting_mm"));
    assert_eq!(cfg.threshold, Some(0.025));
    assert_eq!(cfg.symbol, "will-eth-flip-btc");
}

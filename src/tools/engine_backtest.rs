//! Engine-kind backtest driver (Brecha 1).
//!
//! Runs any `strategy-core` engine (arb_binary, fair_value, fv_momentum,
//! rotation_compounder, arb_hedge, minting_mm) in `ExecutionMode::Backtest`
//! using Binance OHLCV candles normalized to binary-probability space as the
//! data feed.
//!
//! **Data flow**:
//! 1. Fetch `BTCUSDT` 1m candles from Binance for the requested date range
//!    (reuses the same cache already used by the Rhai backtest path).
//! 2. Aggregate to 5-minute windows (same cadence as the Polymarket series).
//! 3. Linearly normalise close prices → YES probability in [0.10, 0.90].
//!    For multi-market engines each slug gets a small independent offset so
//!    prices diverge slightly and rotation/arb logic is exercised.
//! 4. Feed `MarketSnapshot` ticks to the engine and collect `EngineMetrics`.
//! 5. Convert to `BacktestMetrics` and return.

use std::path::Path;
use chrono::Utc;
use strategy_core::{
    engine::StrategyEngine,
    types::{CandleSnap, ExecutionMode, MarketSnapshot, Portfolio},
};
use crate::tools::backtest::{fetch_candles, BacktestMetrics};

// ── Params ────────────────────────────────────────────────────────────────────

pub struct EngineBacktestParams<'a> {
    /// Engine kind: "arb_binary" | "fair_value" | "fv_momentum" |
    ///              "rotation_compounder" | "arb_hedge" | "minting_mm"
    pub kind: &'a str,
    /// Market slugs (comma-separated from `symbol` field).
    pub markets: Vec<String>,
    /// Edge / threshold parameter passed to the engine config.
    pub threshold: Option<f64>,
    pub from_date: &'a str,
    pub to_date: &'a str,
    pub initial_balance: f64,
    pub workspace_dir: &'a Path,
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub async fn run_engine_backtest(params: EngineBacktestParams<'_>) -> BacktestMetrics {
    // 1. Fetch base BTCUSDT 1m candles.
    let raw = match fetch_candles("BTCUSDT", "1m", params.from_date, params.to_date, params.workspace_dir).await {
        Ok(c) if !c.is_empty() => c,
        Ok(_) => return err_metrics("No Binance 1m data for the requested date range. Try a different date range."),
        Err(e) => return err_metrics(&format!("Failed to fetch Binance data: {e}")),
    };

    // 2. Aggregate 1m → 5m.
    let candles: Vec<_> = raw
        .chunks(5)
        .filter(|ch| !ch.is_empty())
        .map(|ch| {
            let close  = ch.last().unwrap().close;
            let open   = ch[0].open;
            let high   = ch.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
            let low    = ch.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
            let volume = ch.iter().map(|c| c.volume).sum::<f64>();
            let ts_ms  = ch[0].open_time_ms;
            (ts_ms, open, high, low, close, volume)
        })
        .collect();

    if candles.is_empty() {
        return err_metrics("Aggregated candles are empty after 5m grouping.");
    }

    // 3. Normalise close → YES probability [0.10, 0.90].
    let closes: Vec<f64> = candles.iter().map(|c| c.4).collect();
    let (min_p, max_p) = closes.iter().fold((f64::MAX, f64::MIN_POSITIVE), |(mn, mx), &c| (mn.min(c), mx.max(c)));
    let range = (max_p - min_p).max(1.0);
    let normalize = |p: f64, offset: f64| -> f64 {
        ((p - min_p) / range * 0.70 + 0.15 + offset).clamp(0.05, 0.95)
    };

    // 4. Build engine and portfolio.
    let mut portfolio = Portfolio::new(params.initial_balance);
    let kind = params.kind;
    let markets = &params.markets;
    let threshold = params.threshold;
    let n_markets = markets.len().max(1);

    // Run the engine via a trait-object so we don't repeat the tick loop.
    macro_rules! run_engine {
        ($eng:expr) => {{
            let mut engine = $eng;
            if engine.initialize(ExecutionMode::Backtest, &portfolio).await.is_err() {
                return err_metrics("Engine initialisation failed.");
            }
            for (idx, (ts_ms, open, high, low, close, volume)) in candles.iter().enumerate() {
                for (m_idx, slug) in markets.iter().enumerate() {
                    // Stagger per-market offsets so multi-market engines see divergent prices.
                    let offset = (m_idx as f64 - (n_markets as f64 - 1.0) / 2.0) * 0.06;
                    let yes = normalize(*close, offset);
                    let snap = MarketSnapshot {
                        market_id: format!("{slug}_yes"),
                        slug: slug.clone(),
                        candle: Some(CandleSnap {
                            open_time_ms: *ts_ms,
                            open:  normalize(*open,  offset),
                            high:  normalize(*high,  offset + 0.02),
                            low:   normalize(*low,   offset - 0.02),
                            close: yes,
                            volume: *volume,
                        }),
                        book: None,
                        timestamp: Utc::now(),
                    };
                    let _ = engine.on_tick(&snap, &mut portfolio).await;
                    let _ = idx; // suppress unused warning in some branches
                }
            }
            engine.finalize(&portfolio).await
        }};
    }

    let metrics = match kind {
        "arb_binary" => {
            use crate::engines::arb_binary::{ArbBinaryConfig, ArbBinaryEngine};
            let cfg = ArbBinaryConfig {
                markets: markets.clone(),
                min_edge_pct: threshold.unwrap_or(0.05),
                max_position_usd: params.initial_balance * 0.25,
                ..Default::default()
            };
            run_engine!(ArbBinaryEngine::new(cfg))
        }
        "fair_value" => {
            use crate::engines::fair_value::{FairValueConfig, FairValueEngine};
            let cfg = FairValueConfig {
                markets: markets.clone(),
                edge_threshold: threshold.unwrap_or(0.005),
                max_position_usd: params.initial_balance * 0.25,
                ..Default::default()
            };
            run_engine!(FairValueEngine::new(cfg))
        }
        "fv_momentum" => {
            use crate::engines::fv_momentum::{FvMomentumConfig, FvMomentumEngine};
            use crate::engines::fair_value::FairValueConfig;
            let fv = FairValueConfig {
                markets: markets.clone(),
                edge_threshold: threshold.unwrap_or(0.005),
                max_position_usd: params.initial_balance * 0.25,
                ..Default::default()
            };
            let cfg = FvMomentumConfig { fv, ..Default::default() };
            run_engine!(FvMomentumEngine::new(cfg))
        }
        "rotation_compounder" => {
            use crate::engines::rotation_compounder::{RotationConfig, RotationCompounderEngine};
            let cfg = RotationConfig {
                markets: markets.clone(),
                max_allocation_pct: 0.60,
                switch_threshold: threshold.unwrap_or(0.001),
                min_position_usd: 10.0,
                stop_loss_pct: 0.40,
                poll_secs: 60,
                sim_days_to_close: 15.0,
            };
            run_engine!(RotationCompounderEngine::new(cfg))
        }
        "arb_hedge" => {
            use crate::engines::arb_hedge::{ArbHedgeConfig, ArbHedgeEngine};
            let cfg = ArbHedgeConfig {
                markets: markets.clone(),
                min_arb_edge: threshold.unwrap_or(0.03),
                hedge_trigger_pct: 0.20,
                max_position_usd: params.initial_balance * 0.25,
                poll_secs: 60,
            };
            run_engine!(ArbHedgeEngine::new(cfg))
        }
        "minting_mm" => {
            use crate::engines::minting_mm::{MintingMmConfig, MintingMmEngine};
            let cfg = MintingMmConfig {
                markets: markets.clone(),
                premium_cents: threshold.unwrap_or(0.03),
                max_cycle_usd: params.initial_balance * 0.25,
                cycle_hours: 24,
                min_spread: 0.04,
                collateral: "0xUSCD".to_string(),
                poll_secs: 60,
                target_apy: 0.40,
            };
            run_engine!(MintingMmEngine::new(cfg))
        }
        other => {
            return err_metrics(&format!("Unknown engine kind '{other}'. Valid: arb_binary, fair_value, fv_momentum, rotation_compounder, arb_hedge, minting_mm"));
        }
    };

    BacktestMetrics {
        total_return_pct:             metrics.total_return_pct,
        sharpe_ratio:                 metrics.sharpe_ratio,
        max_drawdown_pct:             metrics.max_drawdown_pct,
        win_rate_pct:                 metrics.win_rate_pct,
        total_trades:                 metrics.total_trades,
        worst_trades:                 vec![],
        all_trades:                   vec![],
        analysis:                     metrics.analysis,
        avg_token_price:              None,
        correct_direction_pct:        None,
        break_even_win_rate:          None,
        markets_tested:               Some(markets.len() as u32),
        windows_with_real_price:      None,
        windows_with_estimated_price: None,
        historical_data_coverage_pct: None,
        recommended_max_stake_usd:    None,
        flat_debugs:                  vec![],
        position:                     0.0,
        kv_state:                     std::collections::HashMap::new(),
    }
}

// ── Helper ────────────────────────────────────────────────────────────────────

fn err_metrics(msg: &str) -> BacktestMetrics {
    BacktestMetrics {
        analysis: msg.to_string(),
        ..Default::default()
    }
}

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
use chrono::{DateTime, TimeZone, Utc};
use strategy_core::{
    engine::StrategyEngine,
    types::{
        BookLevel, BookSnapshot, CandleSnap, ExecutionMode, MarketSnapshot,
        OrderIntent, Portfolio, Position, Side,
    },
};
use crate::tools::backtest::{fetch_candles, load_ticks_for_range, AllTrade, BacktestMetrics};

// ── Params ────────────────────────────────────────────────────────────────────

pub struct EngineBacktestParams<'a> {
    /// Engine kind: "arb_binary" | "fair_value" | "fv_momentum" |
    ///              "rotation_compounder" | "arb_hedge" | "minting_mm"
    pub kind: &'a str,
    /// Market slugs (comma-separated from `symbol` field).
    pub markets: Vec<String>,
    /// Edge / threshold parameter passed to the engine config.
    pub threshold: Option<f64>,
    /// Per-engine UI overrides (merged over each engine's defaults).
    pub engine_params: Option<serde_json::Value>,
    pub from_date: &'a str,
    pub to_date: &'a str,
    pub initial_balance: f64,
    pub workspace_dir: &'a Path,
}

/// Helper: merge UI engine_params into a typed config by round-tripping through
/// JSON. Fields present in `params` override fields in `base`; missing fields
/// keep the base default.
pub fn merge_params<T: serde::Serialize + serde::de::DeserializeOwned>(
    base: T,
    overrides: Option<&serde_json::Value>,
) -> T {
    let Some(ov) = overrides else { return base };
    let Ok(mut map) = serde_json::to_value(&base)
        .map(|v| v.as_object().cloned().unwrap_or_default())
    else {
        return base;
    };
    if let Some(obj) = ov.as_object() {
        for (k, v) in obj {
            map.insert(k.clone(), v.clone());
        }
    }
    serde_json::from_value(serde_json::Value::Object(map)).unwrap_or(base)
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

    // Run the engine; also collect per-candle balance snapshots for equity curve.
    macro_rules! run_engine {
        ($eng:expr) => {{
            let mut engine = $eng;
            if engine.initialize(ExecutionMode::Backtest, &portfolio).await.is_err() {
                return err_metrics("Engine initialisation failed.");
            }
            let mut snapshots: Vec<(i64, f64)> = Vec::with_capacity(candles.len());
            for (ts_ms, open, high, low, close, volume) in candles.iter() {
                for (m_idx, slug) in markets.iter().enumerate() {
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
                }
                snapshots.push((*ts_ms, portfolio.balance_usdc));
            }
            let metrics = engine.finalize(&portfolio).await;
            (metrics, snapshots)
        }};
    }

    let ov = params.engine_params.as_ref();
    let (metrics, snapshots) = match kind {
        "arb_binary" => {
            use crate::engines::arb_binary::{ArbBinaryConfig, ArbBinaryEngine};
            let base = ArbBinaryConfig {
                markets: markets.clone(),
                min_edge_pct: threshold.unwrap_or(0.05),
                max_position_usd: params.initial_balance * 0.25,
                ..Default::default()
            };
            run_engine!(ArbBinaryEngine::new(merge_params(base, ov)))
        }
        "fair_value" => {
            use crate::engines::fair_value::{FairValueConfig, FairValueEngine};
            let base = FairValueConfig {
                markets: markets.clone(),
                edge_threshold: threshold.unwrap_or(0.005),
                max_position_usd: params.initial_balance * 0.25,
                ..Default::default()
            };
            run_engine!(FairValueEngine::new(merge_params(base, ov)))
        }
        "fv_momentum" => {
            use crate::engines::fv_momentum::{FvMomentumConfig, FvMomentumEngine};
            use crate::engines::fair_value::FairValueConfig;
            let fv_base = FairValueConfig {
                markets: markets.clone(),
                edge_threshold: threshold.unwrap_or(0.005),
                max_position_usd: params.initial_balance * 0.25,
                ..Default::default()
            };
            let base = FvMomentumConfig { fv: fv_base, ..Default::default() };
            run_engine!(FvMomentumEngine::new(merge_params(base, ov)))
        }
        "rotation_compounder" => {
            use crate::engines::rotation_compounder::{RotationConfig, RotationCompounderEngine};
            let base = RotationConfig {
                markets: markets.clone(),
                max_allocation_pct: 0.60,
                switch_threshold: threshold.unwrap_or(0.001),
                min_position_usd: 10.0,
                stop_loss_pct: 0.40,
                poll_secs: 60,
                sim_days_to_close: 15.0,
            };
            run_engine!(RotationCompounderEngine::new(merge_params(base, ov)))
        }
        "arb_hedge" => {
            use crate::engines::arb_hedge::{ArbHedgeConfig, ArbHedgeEngine};
            let base = ArbHedgeConfig {
                markets: markets.clone(),
                min_arb_edge: threshold.unwrap_or(0.03),
                hedge_trigger_pct: 0.20,
                max_position_usd: params.initial_balance * 0.25,
                poll_secs: 60,
            };
            run_engine!(ArbHedgeEngine::new(merge_params(base, ov)))
        }
        "minting_mm" => {
            use crate::engines::minting_mm::{MintingMmConfig, MintingMmEngine};
            let base = MintingMmConfig {
                markets: markets.clone(),
                premium_cents: threshold.unwrap_or(0.03),
                max_cycle_usd: params.initial_balance * 0.25,
                cycle_hours: 24,
                min_spread: 0.04,
                collateral: "0xUSCD".to_string(),
                poll_secs: 60,
                target_apy: 0.40,
            };
            run_engine!(MintingMmEngine::new(merge_params(base, ov)))
        }
        other => {
            return err_metrics(&format!("Unknown engine kind '{other}'. Valid: arb_binary, fair_value, fv_momentum, rotation_compounder, arb_hedge, minting_mm"));
        }
    };

    // Build equity-curve trades from per-candle balance snapshots.
    let initial = params.initial_balance;
    let all_trades: Vec<AllTrade> = snapshots
        .into_iter()
        .map(|(ts_ms, bal)| {
            let dt: DateTime<Utc> = Utc.timestamp_millis_opt(ts_ms).single().unwrap_or_else(Utc::now);
            AllTrade {
                timestamp: dt.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                side: "equity".to_string(),
                price: bal / initial,
                size: 0.0,
                pnl: bal - initial,
                balance: bal,
                debug: None,
            }
        })
        .collect();

    BacktestMetrics {
        total_return_pct:             metrics.total_return_pct,
        sharpe_ratio:                 metrics.sharpe_ratio,
        max_drawdown_pct:             metrics.max_drawdown_pct,
        win_rate_pct:                 metrics.win_rate_pct,
        total_trades:                 metrics.total_trades,
        worst_trades:                 vec![],
        all_trades,
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

// ── CLOB 1 HZ tick replay for engine kinds ────────────────────────────────────

/// Params for replaying recorded CLOB tick data through a strategy-core engine.
///
/// Mirrors [`EngineBacktestParams`] but consumes a tick slug (e.g. "btc_5m")
/// recorded by the [`crate::tick_recorder::TickRecorder`]. The engine receives
/// real Polymarket YES/NO best-bid/ask prices instead of synthetic candles.
pub struct EngineClobBacktestParams<'a> {
    pub kind: &'a str,
    /// Tick slug to replay (e.g. "btc_5m"). Used as the only `markets` entry
    /// passed to the engine config.
    pub slug: &'a str,
    pub threshold: Option<f64>,
    pub engine_params: Option<serde_json::Value>,
    pub from_date: &'a str,
    pub to_date: &'a str,
    pub initial_balance: f64,
    pub fee_pct: f64,
    pub workspace_dir: &'a Path,
}

/// Replay recorded CLOB tick JSONL files through a strategy-core engine.
///
/// Each tick is converted into a [`BookSnapshot`] (using recorded YES/NO
/// best-bid/ask prices) and dispatched to `engine.on_book(...)`. Buy intents
/// open a synthetic position priced at the recorded ask; positions are settled
/// at every window close (`window_secs_left == 0`) using `binance_price` vs
/// `window_open_price` as the YES/NO outcome — same resolution model used by
/// the Rhai `on_tick` backtester.
pub async fn run_engine_clob_1hz_backtest(params: EngineClobBacktestParams<'_>) -> BacktestMetrics {
    // 1. Load recorded ticks for the slug.
    let ticks = match load_ticks_for_range(
        params.workspace_dir,
        params.slug,
        params.from_date,
        params.to_date,
    ) {
        Ok(t) if !t.is_empty() => t,
        Ok(_) => return err_metrics(&format!(
            "No recorded ticks for slug '{}' in {} → {}. Start the tick recorder first.",
            params.slug, params.from_date, params.to_date,
        )),
        Err(e) => return err_metrics(&format!("Failed to load ticks: {e}")),
    };

    let kind = params.kind;
    let markets = vec![params.slug.to_string()];
    let threshold = params.threshold;
    let initial = params.initial_balance;
    let fee_pct = params.fee_pct;
    let yes_token = format!("{}_yes", params.slug);
    let no_token  = format!("{}_no",  params.slug);

    let mut portfolio = Portfolio::new(initial);
    let ov = params.engine_params.as_ref();

    macro_rules! run_engine {
        ($eng:expr) => {{
            let mut engine = $eng;
            if engine.initialize(ExecutionMode::Backtest, &portfolio).await.is_err() {
                return err_metrics("Engine initialisation failed.");
            }

            let mut snapshots: Vec<(i64, f64)> = Vec::with_capacity(ticks.len());
            let mut all_trades: Vec<AllTrade> = Vec::new();
            // Window-resolution state — mirrors the on_tick CLOB backtester so
            // engine intents settle the same way Rhai bets do.
            let mut window_ts: i64 = 0;
            let mut window_open_price: f64 = 0.0;
            let mut current_side: Option<Side> = None;
            let mut current_token: Option<String> = None;
            let mut current_size_tokens: f64 = 0.0;
            let mut current_entry_price: f64 = 0.0;
            let mut current_stake: f64 = 0.0;

            for tick in ticks.iter() {
                // Track window open price.
                if tick.window_ts != window_ts {
                    window_ts = tick.window_ts;
                    window_open_price = tick.binance_price;
                }
                if window_open_price <= 0.0 && tick.binance_price > 0.0 {
                    window_open_price = tick.binance_price;
                }

                // Settle at window close.
                if tick.window_secs_left == 0 {
                    if let (Some(side), Some(token_id)) = (&current_side, &current_token) {
                        let yes_won = tick.binance_price > window_open_price;
                        let won = match side {
                            Side::Yes => yes_won,
                            Side::No  => !yes_won,
                        };
                        let pnl = if won {
                            // Token pays $1 each on win.
                            let payout = current_size_tokens;
                            portfolio.balance_usdc += payout;
                            portfolio.realized_pnl += payout - current_stake;
                            payout - current_stake
                        } else {
                            portfolio.realized_pnl -= current_stake;
                            -current_stake
                        };
                        portfolio.positions.remove(token_id);

                        let ts_str = chrono::DateTime::from_timestamp_millis(tick.ts_ms)
                            .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
                            .unwrap_or_else(|| tick.ts_ms.to_string());
                        all_trades.push(AllTrade {
                            timestamp: ts_str,
                            side: match side { Side::Yes => "yes".to_string(), Side::No => "no".to_string() },
                            price: current_entry_price,
                            size:  current_stake,
                            pnl,
                            balance: portfolio.balance_usdc,
                            debug: None,
                        });
                    }
                    current_side = None;
                    current_token = None;
                    current_size_tokens = 0.0;
                    current_entry_price = 0.0;
                    current_stake = 0.0;
                    window_open_price = 0.0;
                }

                // Build a real BookSnapshot from the recorded tick. Engines
                // gate on best_ask/best_bid so we use the recorded values
                // verbatim and approximate depth (the recorder doesn't store
                // it). When YES/NO bid/ask is missing we skip the tick rather
                // than feeding zeros that would trigger spurious arbitrage.
                if tick.yes_ask <= 0.0 || tick.no_ask <= 0.0 {
                    continue;
                }
                let snap = BookSnapshot {
                    market_id: yes_token.clone(),
                    slug: params.slug.to_string(),
                    yes: BookLevel {
                        best_ask: tick.yes_ask,
                        best_bid: if tick.yes_bid > 0.0 { tick.yes_bid } else { tick.yes_ask - 0.01 },
                        ask_depth_usd: 1000.0,
                        bid_depth_usd: 1000.0,
                    },
                    no: BookLevel {
                        best_ask: tick.no_ask,
                        best_bid: if tick.no_bid > 0.0 { tick.no_bid } else { tick.no_ask - 0.01 },
                        ask_depth_usd: 1000.0,
                        bid_depth_usd: 1000.0,
                    },
                    timestamp: chrono::DateTime::from_timestamp_millis(tick.ts_ms).unwrap_or_else(Utc::now),
                    meta: Default::default(),
                };

                let intents = match engine.on_book(&snap, &mut portfolio).await {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                // Translate intents → simulated fills. Only one open position
                // per window (matches the Rhai on_tick model and prevents the
                // engines from double-betting the same window).
                if current_side.is_some() {
                    continue;
                }
                for intent in intents {
                    let OrderIntent::Buy { token_id, side, size_usd, limit_price } = intent else { continue };
                    let ask = match side { Side::Yes => tick.yes_ask, Side::No => tick.no_ask };
                    if ask <= 0.0 || ask >= 1.0 { continue; }
                    if let Some(lp) = limit_price {
                        if lp + 1e-9 < ask { continue; }
                    }
                    let stake = size_usd.min(portfolio.balance_usdc).max(0.0);
                    let stake = stake * (1.0 - fee_pct / 100.0);
                    if stake <= 0.01 { continue; }

                    portfolio.balance_usdc -= stake;
                    let size_tokens = stake / ask;
                    let actual_token = match side {
                        Side::Yes => yes_token.clone(),
                        Side::No  => no_token.clone(),
                    };
                    portfolio.positions.insert(actual_token.clone(), Position {
                        token_id: actual_token.clone(),
                        side: side.clone(),
                        size: size_tokens,
                        entry_price: ask,
                        current_price: ask,
                        unrealized_pnl: 0.0,
                        opened_at: snap.timestamp,
                    });
                    let _ = token_id;
                    current_side = Some(side);
                    current_token = Some(actual_token);
                    current_size_tokens = size_tokens;
                    current_entry_price = ask;
                    current_stake = stake;
                    break;
                }

                snapshots.push((tick.ts_ms, portfolio.balance_usdc));
            }

            let metrics = engine.finalize(&portfolio).await;
            (metrics, snapshots, all_trades)
        }};
    }

    let (metrics, snapshots, trades) = match kind {
        "arb_binary" => {
            use crate::engines::arb_binary::{ArbBinaryConfig, ArbBinaryEngine};
            let base = ArbBinaryConfig {
                markets: markets.clone(),
                min_edge_pct: threshold.unwrap_or(0.05),
                max_position_usd: initial * 0.25,
                fee_pct: fee_pct / 100.0,
                ..Default::default()
            };
            run_engine!(ArbBinaryEngine::new(merge_params(base, ov)))
        }
        "fair_value" => {
            use crate::engines::fair_value::{FairValueConfig, FairValueEngine};
            let base = FairValueConfig {
                markets: markets.clone(),
                edge_threshold: threshold.unwrap_or(0.005),
                max_position_usd: initial * 0.25,
                ..Default::default()
            };
            run_engine!(FairValueEngine::new(merge_params(base, ov)))
        }
        "fv_momentum" => {
            use crate::engines::fv_momentum::{FvMomentumConfig, FvMomentumEngine};
            use crate::engines::fair_value::FairValueConfig;
            let fv_base = FairValueConfig {
                markets: markets.clone(),
                edge_threshold: threshold.unwrap_or(0.005),
                max_position_usd: initial * 0.25,
                ..Default::default()
            };
            let base = FvMomentumConfig { fv: fv_base, ..Default::default() };
            run_engine!(FvMomentumEngine::new(merge_params(base, ov)))
        }
        "rotation_compounder" => {
            use crate::engines::rotation_compounder::{RotationConfig, RotationCompounderEngine};
            let base = RotationConfig {
                markets: markets.clone(),
                max_allocation_pct: 0.60,
                switch_threshold: threshold.unwrap_or(0.001),
                min_position_usd: 10.0,
                stop_loss_pct: 0.40,
                poll_secs: 60,
                sim_days_to_close: 15.0,
            };
            run_engine!(RotationCompounderEngine::new(merge_params(base, ov)))
        }
        "arb_hedge" => {
            use crate::engines::arb_hedge::{ArbHedgeConfig, ArbHedgeEngine};
            let base = ArbHedgeConfig {
                markets: markets.clone(),
                min_arb_edge: threshold.unwrap_or(0.03),
                hedge_trigger_pct: 0.20,
                max_position_usd: initial * 0.25,
                poll_secs: 60,
            };
            run_engine!(ArbHedgeEngine::new(merge_params(base, ov)))
        }
        "minting_mm" => {
            use crate::engines::minting_mm::{MintingMmConfig, MintingMmEngine};
            let base = MintingMmConfig {
                markets: markets.clone(),
                premium_cents: threshold.unwrap_or(0.03),
                max_cycle_usd: initial * 0.25,
                cycle_hours: 24,
                min_spread: 0.04,
                collateral: "0xUSCD".to_string(),
                poll_secs: 60,
                target_apy: 0.40,
            };
            run_engine!(MintingMmEngine::new(merge_params(base, ov)))
        }
        other => return err_metrics(&format!(
            "Unknown engine kind '{other}' for CLOB 1 HZ backtest."
        )),
    };

    // Build equity curve from per-tick balance snapshots, sampled coarsely so
    // the response payload doesn't explode (one point every ~5 minutes).
    let initial_bal = initial;
    let stride = (snapshots.len() / 600).max(1);
    let mut equity: Vec<AllTrade> = snapshots
        .iter()
        .step_by(stride)
        .map(|(ts_ms, bal)| {
            let dt: DateTime<Utc> = Utc.timestamp_millis_opt(*ts_ms).single().unwrap_or_else(Utc::now);
            AllTrade {
                timestamp: dt.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                side: "equity".to_string(),
                price: bal / initial_bal,
                size: 0.0,
                pnl: bal - initial_bal,
                balance: *bal,
                debug: None,
            }
        })
        .collect();
    equity.extend(trades);

    BacktestMetrics {
        total_return_pct:             metrics.total_return_pct,
        sharpe_ratio:                 metrics.sharpe_ratio,
        max_drawdown_pct:             metrics.max_drawdown_pct,
        win_rate_pct:                 metrics.win_rate_pct,
        total_trades:                 metrics.total_trades,
        worst_trades:                 vec![],
        all_trades:                   equity,
        analysis:                     metrics.analysis,
        avg_token_price:              None,
        correct_direction_pct:        None,
        break_even_win_rate:          None,
        markets_tested:               Some(1),
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

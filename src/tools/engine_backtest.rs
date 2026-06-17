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

/// Whether to settle windows lacking an official resolution via the Binance close.
/// Default false (= VOID such windows) — that fallback is a lookahead vector. Set
/// NV_ALLOW_BINANCE_FALLBACK=1 to restore the old behaviour. Mirrors backtest.rs.
fn engine_allow_binance_fallback() -> bool {
    std::env::var("NV_ALLOW_BINANCE_FALLBACK")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

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
                    if let (Some(_side), Some(token_id)) = (&current_side, &current_token) {
                        if tick.window_yes_won.is_none() && !engine_allow_binance_fallback() {
                            // No official resolution → VOID: refund stake, no trade recorded
                            // (never settle on the Binance close — lookahead vector). Matches
                            // the default in the on_candle / on_tick backtest paths.
                            portfolio.balance_usdc += current_stake;
                            portfolio.positions.remove(token_id);
                            current_side = None; current_token = None;
                            current_size_tokens = 0.0; current_entry_price = 0.0; current_stake = 0.0;
                            window_open_price = 0.0;
                            continue;
                        }
                    }
                    if let (Some(side), Some(token_id)) = (&current_side, &current_token) {
                        let yes_won = match tick.window_yes_won {
                            Some(official) => official,
                            None => tick.binance_price > window_open_price, // only with escape hatch
                        };
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

// ── clob_events: native engines on the sub-second event stream (Fase E) ────────

/// Params for replaying the millisecond `to-events` stream through a strategy-core
/// engine. Unlike [`EngineClobBacktestParams`] (1Hz ticks, NO book derived as
/// `1-yes`, fixed depth, Binance-only resolution), this feeds engines a REAL
/// two-sided book (YES and NO reconstructed independently from events) at ms
/// resolution and settles with the OFFICIAL Polymarket resolution.
pub struct EngineClobEventsParams<'a> {
    pub kind: &'a str,
    /// Event-stream slug (e.g. "btc_5m_ev").
    pub slug: &'a str,
    pub threshold: Option<f64>,
    pub engine_params: Option<serde_json::Value>,
    pub from_date: &'a str,
    pub to_date: &'a str,
    pub initial_balance: f64,
    pub fee_pct: f64,
    pub workspace_dir: &'a Path,
}

/// Replay the ms event stream through a strategy-core engine via `on_book`.
///
/// Each book event updates a live two-sided book; the engine sees a real
/// [`BookSnapshot`] (YES & NO independent bid/ask). Buy intents open one
/// position per window; positions settle at the window boundary using the
/// official `window_yes_won` from the stream's meta header (Binance fallback
/// only when a window has no official resolution).
pub async fn run_engine_clob_events_backtest(params: EngineClobEventsParams<'_>) -> BacktestMetrics {
    use crate::tools::backtest::{load_events_for_range, event_window_ts, EvKind};

    let stream = match load_events_for_range(
        params.workspace_dir, params.slug, params.from_date, params.to_date,
    ) {
        Ok(s) if !s.events.is_empty() => s,
        Ok(_) => return err_metrics(&format!(
            "No events for slug '{}' in {} → {}. Generate it with `to-events`.",
            params.slug, params.from_date, params.to_date)),
        Err(e) => return err_metrics(&format!("Failed to load events: {e}")),
    };

    let window_secs = stream.window_minutes * 60;
    // window_ts → official resolution, so each window settles with its OWN cid.
    let mut official_by_window: std::collections::HashMap<i64, bool> = std::collections::HashMap::new();
    for ev in &stream.events {
        if let Some(yw) = stream.markets.get(&ev.cid).and_then(|m| m.yes_won) {
            official_by_window.entry(event_window_ts(ev.ts_ms, window_secs)).or_insert(yw);
        }
    }

    let kind = params.kind;
    let markets = vec![params.slug.to_string()];
    let threshold = params.threshold;
    let initial = params.initial_balance;
    let fee_pct = params.fee_pct;
    let yes_token = format!("{}_yes", params.slug);
    let no_token = format!("{}_no", params.slug);
    let mut portfolio = Portfolio::new(initial);
    let ov = params.engine_params.as_ref();

    macro_rules! run_engine {
        ($eng:expr) => {{
            let mut engine = $eng;
            if engine.initialize(ExecutionMode::Backtest, &portfolio).await.is_err() {
                return err_metrics("Engine initialisation failed.");
            }
            // Live two-sided book reconstructed from events.
            let (mut yes_bid, mut yes_ask, mut no_bid, mut no_ask, mut binance) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
            let mut snapshots: Vec<(i64, f64)> = Vec::new();
            let mut all_trades: Vec<AllTrade> = Vec::new();
            let mut cur_window: i64 = 0;
            let mut window_open_price: f64 = 0.0;
            let mut pos_side: Option<Side> = None;
            let mut pos_token: Option<String> = None;
            let mut pos_size_tokens = 0.0f64;
            let mut pos_entry = 0.0f64;
            let mut pos_stake = 0.0f64;

            for ev in stream.events.iter() {
                // 1. Update live book from this event.
                if ev.kind == EvKind::Book {
                    match ev.token {
                        0 => { yes_bid = ev.bid; yes_ask = ev.ask; }
                        1 => { no_bid = ev.bid; no_ask = ev.ask; }
                        _ => {}
                    }
                }
                if ev.binance_price > 0.0 { binance = ev.binance_price; }

                let wts = event_window_ts(ev.ts_ms, window_secs);

                // 2. Window boundary → settle the open position with the CLOSING
                //    window's official resolution.
                if wts != cur_window && cur_window != 0 {
                    if let (Some(_side), Some(token_id)) = (&pos_side, &pos_token) {
                        if official_by_window.get(&cur_window).is_none() && !engine_allow_binance_fallback() {
                            // No official resolution → VOID (refund stake, no trade). Default;
                            // NV_ALLOW_BINANCE_FALLBACK=1 restores the old Binance-close settle.
                            portfolio.balance_usdc += pos_stake;
                            portfolio.positions.remove(token_id);
                            pos_side = None; pos_token = None; pos_size_tokens = 0.0;
                            pos_entry = 0.0; pos_stake = 0.0; window_open_price = 0.0;
                            if wts != cur_window { cur_window = wts; }
                            continue;
                        }
                    }
                    if let (Some(side), Some(token_id)) = (&pos_side, &pos_token) {
                        let yes_won = match official_by_window.get(&cur_window) {
                            Some(o) => *o,
                            None => binance > window_open_price, // only with escape hatch
                        };
                        let won = match side { Side::Yes => yes_won, Side::No => !yes_won };
                        let pnl = if won {
                            let payout = pos_size_tokens;
                            portfolio.balance_usdc += payout;
                            portfolio.realized_pnl += payout - pos_stake;
                            payout - pos_stake
                        } else {
                            portfolio.realized_pnl -= pos_stake;
                            -pos_stake
                        };
                        portfolio.positions.remove(token_id);
                        let ts_str = chrono::DateTime::from_timestamp_millis(ev.ts_ms)
                            .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
                            .unwrap_or_else(|| ev.ts_ms.to_string());
                        all_trades.push(AllTrade {
                            timestamp: ts_str,
                            side: match side { Side::Yes => "yes".into(), Side::No => "no".into() },
                            price: pos_entry, size: pos_stake, pnl,
                            balance: portfolio.balance_usdc, debug: None,
                        });
                    }
                    pos_side = None; pos_token = None; pos_size_tokens = 0.0;
                    pos_entry = 0.0; pos_stake = 0.0; window_open_price = 0.0;
                }
                if wts != cur_window { cur_window = wts; }
                if window_open_price <= 0.0 && binance > 0.0 { window_open_price = binance; }

                // 3. Feed the engine a real two-sided book (skip until both sides known).
                if yes_ask <= 0.0 || no_ask <= 0.0 { continue; }
                let snap = BookSnapshot {
                    market_id: yes_token.clone(),
                    slug: params.slug.to_string(),
                    yes: BookLevel {
                        best_ask: yes_ask,
                        best_bid: if yes_bid > 0.0 { yes_bid } else { (yes_ask - 0.01).max(0.0) },
                        ask_depth_usd: 1000.0, bid_depth_usd: 1000.0,
                    },
                    no: BookLevel {
                        best_ask: no_ask,
                        best_bid: if no_bid > 0.0 { no_bid } else { (no_ask - 0.01).max(0.0) },
                        ask_depth_usd: 1000.0, bid_depth_usd: 1000.0,
                    },
                    timestamp: chrono::DateTime::from_timestamp_millis(ev.ts_ms).unwrap_or_else(Utc::now),
                    meta: Default::default(),
                };
                let intents = match engine.on_book(&snap, &mut portfolio).await {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if pos_side.is_some() { continue; } // one position per window
                for intent in intents {
                    let OrderIntent::Buy { token_id, side, size_usd, limit_price } = intent else { continue };
                    let ask = match side { Side::Yes => yes_ask, Side::No => no_ask };
                    if ask <= 0.0 || ask >= 1.0 { continue; }
                    if let Some(lp) = limit_price { if lp + 1e-9 < ask { continue; } }
                    let stake = (size_usd.min(portfolio.balance_usdc).max(0.0)) * (1.0 - fee_pct / 100.0);
                    if stake <= 0.01 { continue; }
                    portfolio.balance_usdc -= stake;
                    let size_tokens = stake / ask;
                    let actual_token = match side { Side::Yes => yes_token.clone(), Side::No => no_token.clone() };
                    portfolio.positions.insert(actual_token.clone(), Position {
                        token_id: actual_token.clone(), side: side.clone(), size: size_tokens,
                        entry_price: ask, current_price: ask, unrealized_pnl: 0.0,
                        opened_at: snap.timestamp,
                    });
                    let _ = token_id;
                    pos_side = Some(side); pos_token = Some(actual_token);
                    pos_size_tokens = size_tokens; pos_entry = ask; pos_stake = stake;
                    break;
                }
                snapshots.push((ev.ts_ms, portfolio.balance_usdc));
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
        other => return err_metrics(&format!(
            "Engine kind '{other}' is not supported on clob_events. Use arb_binary, \
             fair_value, fv_momentum, or arb_hedge (book-driven engines)."
        )),
    };

    let initial_bal = initial;
    let stride = (snapshots.len() / 600).max(1);
    let mut equity: Vec<AllTrade> = snapshots.iter().step_by(stride).map(|(ts_ms, bal)| {
        let dt: DateTime<Utc> = Utc.timestamp_millis_opt(*ts_ms).single().unwrap_or_else(Utc::now);
        AllTrade {
            timestamp: dt.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            side: "equity".to_string(), price: bal / initial_bal, size: 0.0,
            pnl: bal - initial_bal, balance: *bal, debug: None,
        }
    }).collect();
    equity.extend(trades);

    BacktestMetrics {
        total_return_pct: metrics.total_return_pct,
        sharpe_ratio: metrics.sharpe_ratio,
        max_drawdown_pct: metrics.max_drawdown_pct,
        win_rate_pct: metrics.win_rate_pct,
        total_trades: metrics.total_trades,
        all_trades: equity,
        analysis: format!("{}\n[clob_events: real two-sided book, official resolution]", metrics.analysis),
        markets_tested: Some(1),
        ..Default::default()
    }
}

// ── Fase D: MAKER fill model on the event stream (rewards_maker backtest) ──────

/// Params for the maker backtest. Mirrors the live `rewards_maker` engine:
/// keep a bilateral resting quote at `mid ± offset`, re-center on drift, and
/// measure fills / adverse selection / eligible uptime against the real event
/// stream — the honest backtest the manual rewards pilot lacked.
pub struct MakerBacktestParams<'a> {
    /// Event-stream slug (e.g. "btc_5m_ev").
    pub slug: &'a str,
    /// Cents inside the mid each leg rests (mirrors live DEFAULT_OFFSET_C = 1¢).
    pub offset_cents: f64,
    /// Re-center the quote when the mid drifts more than this (absolute price).
    pub reprice_threshold: f64,
    /// USD per side per quote.
    pub size_usd: f64,
    pub from_date: &'a str,
    pub to_date: &'a str,
    pub initial_balance: f64,
    pub workspace_dir: &'a Path,
}

/// Backtest a bilateral resting-quote maker over the ms event stream.
///
/// Fill model: a resting BUY at price `q` on a token fills when a `trade` event
/// on that token prints at price ≤ `q` (someone hit our bid) — top-of-book queue
/// approximation (we assume our quote is at the front when it's the best price,
/// conservative the other way). Adverse selection is measured as the YES-mid
/// move over the next `ADVERSE_HORIZON_MS` after each fill (a fill right before
/// the mid runs away from us is a bad fill). Eligible uptime = fraction of book
/// events where BOTH legs rest within `reprice_threshold` of the live mid.
pub async fn run_maker_backtest(params: MakerBacktestParams<'_>) -> BacktestMetrics {
    use crate::tools::backtest::{load_events_for_range, EvKind};
    const ADVERSE_HORIZON_MS: i64 = 10_000;

    let stream = match load_events_for_range(
        params.workspace_dir, params.slug, params.from_date, params.to_date,
    ) {
        Ok(s) if !s.events.is_empty() => s,
        Ok(_) => return err_metrics(&format!(
            "No events for slug '{}'. Generate it with `to-events`.", params.slug)),
        Err(e) => return err_metrics(&format!("Failed to load events: {e}")),
    };

    let offset = params.offset_cents / 100.0;
    let mut yes_bid = 0.0f64;
    let mut yes_ask = 0.0f64;
    // Our two resting quotes (BUY YES at yes_q, BUY NO at no_q). None = not posted.
    let mut yes_q: Option<f64> = None;
    let mut no_q: Option<f64> = None;
    let mut ref_mid = 0.0f64; // mid where the current pair was posted

    let mut fills = 0u32;
    let mut yes_fills = 0u32;
    let mut no_fills = 0u32;
    let mut adverse_fills = 0u32; // fills where mid moved against us within horizon
    let mut eligible_events = 0u64;
    let mut total_book_events = 0u64;
    // Pending adverse-selection checks: (fill_ts_ms, mid_at_fill, side_is_yes).
    let mut pending_adverse: Vec<(i64, f64, bool)> = Vec::new();
    let mut all_trades: Vec<AllTrade> = Vec::new();

    let ts_str = |ms: i64| chrono::DateTime::from_timestamp_millis(ms)
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_else(|| ms.to_string());

    for ev in stream.events.iter() {
        // Maintain the live YES book (the maker quotes around the YES mid).
        if ev.kind == EvKind::Book && ev.token == 0 {
            if ev.bid > 0.0 { yes_bid = ev.bid; }
            if ev.ask > 0.0 { yes_ask = ev.ask; }
        }
        let mid = if yes_bid > 0.0 && yes_ask > 0.0 { (yes_bid + yes_ask) / 2.0 } else { 0.0 };

        // Resolve pending adverse-selection checks whose horizon elapsed.
        if mid > 0.0 {
            pending_adverse.retain(|&(fill_ts, mid_at_fill, side_is_yes)| {
                if ev.ts_ms - fill_ts < ADVERSE_HORIZON_MS { return true; }
                // YES fill is adverse if the YES mid FELL (we bought YES, it dropped);
                // NO fill is adverse if the YES mid ROSE (NO value fell).
                let adverse = if side_is_yes { mid < mid_at_fill } else { mid > mid_at_fill };
                if adverse { adverse_fills += 1; }
                false
            });
        }

        if ev.kind == EvKind::Book {
            if mid <= 0.0 { continue; }
            total_book_events += 1;
            // Re-center on drift (mirrors the live engine).
            if ref_mid > 0.0 && (mid - ref_mid).abs() > params.reprice_threshold {
                yes_q = None; no_q = None;
            }
            // Ensure both legs rest. YES quote = mid - offset; NO quote (in YES
            // terms) means a BUY NO at (1-mid)-offset → fills if YES ask rises to
            // 1-((1-mid)-offset) = mid+offset, i.e. tracked via the YES book.
            if yes_q.is_none() { yes_q = Some(((mid - offset) * 100.0).round() / 100.0); }
            if no_q.is_none()  { no_q  = Some(((mid - offset) * 100.0).round() / 100.0); } // symmetric in ¢
            if yes_q.is_some() && no_q.is_some() { ref_mid = mid; }
            // Eligible iff both legs sit within reprice_threshold of the live mid.
            let elig = yes_q.map(|q| (mid - offset - q).abs() <= params.reprice_threshold).unwrap_or(false)
                    && no_q.is_some();
            if elig { eligible_events += 1; }
        } else if ev.kind == EvKind::Trade && mid > 0.0 {
            // A trade on the YES token at price p:
            //  - a SELL (taker hit the bid) at p ≤ our yes_q fills our BUY YES.
            //  - a BUY  (taker lifted the ask) implies NO-side pressure; we model
            //    the NO leg fill when the YES ask trades up to ≥ 1 - no_q.
            let p = ev.ask; // trade price stored in both bid/ask for trade events
            if ev.token == 0 {
                if let Some(q) = yes_q {
                    if p <= q + 1e-9 && p > 0.02 {
                        fills += 1; yes_fills += 1;
                        pending_adverse.push((ev.ts_ms, mid, true));
                        all_trades.push(AllTrade {
                            timestamp: ts_str(ev.ts_ms), side: "maker_yes".into(),
                            price: q, size: params.size_usd, pnl: 0.0,
                            balance: params.initial_balance, debug: None,
                        });
                        yes_q = None; // leg consumed, re-post next book event
                    }
                }
            } else if ev.token == 1 {
                if let Some(q) = no_q {
                    if p <= q + 1e-9 && p > 0.02 {
                        fills += 1; no_fills += 1;
                        pending_adverse.push((ev.ts_ms, mid, false));
                        all_trades.push(AllTrade {
                            timestamp: ts_str(ev.ts_ms), side: "maker_no".into(),
                            price: q, size: params.size_usd, pnl: 0.0,
                            balance: params.initial_balance, debug: None,
                        });
                        no_q = None;
                    }
                }
            }
        }
    }

    let elig_pct = if total_book_events > 0 {
        eligible_events as f64 / total_book_events as f64 * 100.0
    } else { 0.0 };
    let adverse_pct = if fills > 0 { adverse_fills as f64 / fills as f64 * 100.0 } else { 0.0 };

    let analysis = format!(
        "MAKER backtest ({} events over {}→{}): {} fills ({} YES / {} NO). \
         Eligible uptime {:.1}% (both legs within {:.0}¢ of mid). \
         Adverse selection {:.1}% of fills (mid moved against us within {}s).\n\
         Note: top-of-book queue approximation — a resting BUY fills when a trade \
         prints at/through its price. Rewards earnings depend on the Polymarket \
         reward pool (not modeled here); this measures FILL QUALITY + UPTIME, the \
         two things the manual pilot got wrong. Low adverse% + high eligible% = \
         the market is safe to provide liquidity in.",
        stream.events.len(), params.from_date, params.to_date,
        fills, yes_fills, no_fills, elig_pct, params.offset_cents, adverse_pct,
        ADVERSE_HORIZON_MS / 1000,
    );

    let mut extra = std::collections::HashMap::new();
    extra.insert("eligible_uptime_pct".to_string(), elig_pct);
    extra.insert("adverse_selection_pct".to_string(), adverse_pct);
    extra.insert("yes_fills".to_string(), yes_fills as f64);
    extra.insert("no_fills".to_string(), no_fills as f64);

    BacktestMetrics {
        total_return_pct: 0.0, // rewards accrue off-book; not P&L-modeled
        win_rate_pct: 100.0 - adverse_pct, // "good fills" proxy
        total_trades: fills,
        all_trades,
        analysis,
        markets_tested: Some(1),
        kv_state: extra,
        ..Default::default()
    }
}

// ── Helper ────────────────────────────────────────────────────────────────────

fn err_metrics(msg: &str) -> BacktestMetrics {
    BacktestMetrics {
        analysis: msg.to_string(),
        ..Default::default()
    }
}

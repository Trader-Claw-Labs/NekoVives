//! Event-driven backtest engine for strategies that react to funding cycles,
//! migrations, and other discrete market events (not just candles).
//!
//! Supports the `on_event(ctx, event)` Rhai entry point alongside the traditional
//! `on_candle(ctx)`.  The engine consumes a chronologically-sorted `BacktestEvent`
//! stream, maintains multi-venue position state, and produces `BacktestMetrics`.

use crate::tools::backtest::{AllTrade, BacktestMetrics, Candle, WorstTrade};
use crate::tools::historical_data::BacktestEvent;
use rhai::Engine;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// ── Internal state ───────────────────────────────────────────────────

#[derive(Clone, Default)]
struct VenuePos {
    qty: f64,        // + = long, − = short
    entry_price: f64,
    size_usd: f64,   // notional at entry
}

#[derive(Clone)]
struct EventState {
    balance: f64,
    positions: HashMap<(String, String), VenuePos>, // (venue, symbol)
    trades: Vec<EventTrade>,
    kv: HashMap<String, f64>,
    latest_funding_apr: HashMap<(String, String), f64>,
    candle_history: Vec<Candle>,
}

#[derive(Clone)]
struct EventTrade {
    timestamp: String,
    side: String,
    venue: String,
    symbol: String,
    price: f64,
    size: f64,
    pnl: f64,
}

// ── Main entry point ─────────────────────────────────────────────────

/// Run an event-driven backtest.
///
/// * `script_content` – Rhai source (must declare `on_event`, `on_candle`, or both).
/// * `events`         – chronological timeline (`BacktestEvent` stream).
/// * `initial_balance` – starting capital.
/// * `fee_pct`        – taker fee % per leg.
pub fn run_event_backtest(
    script_content: String,
    events: Vec<BacktestEvent>,
    initial_balance: f64,
    fee_pct: f64,
) -> anyhow::Result<BacktestMetrics> {
    if events.is_empty() {
        return Err(anyhow::anyhow!("Empty event timeline"));
    }

    // Detect declared entry points
    let check = Engine::new();
    let has_on_candle = match check.compile(&script_content) {
        Ok(ast) => ast.iter_functions().any(|f| f.name == "on_candle"),
        Err(e) => return Err(anyhow::anyhow!("Script compile error: {e}")),
    };
    let has_on_event = match check.compile(&script_content) {
        Ok(ast) => ast.iter_functions().any(|f| f.name == "on_event"),
        Err(_) => false,
    };
    if !has_on_candle && !has_on_event {
        return Err(anyhow::anyhow!(
            "Event-driven backtest requires on_event(ctx, event) or on_candle(ctx)"
        ));
    }

    let state = Arc::new(Mutex::new(EventState {
        balance: initial_balance,
        positions: HashMap::new(),
        trades: Vec::new(),
        kv: HashMap::new(),
        latest_funding_apr: HashMap::new(),
        candle_history: Vec::new(),
    }));

    let mut portfolio_values: Vec<f64> = vec![initial_balance];
    let mut peak = initial_balance;
    let mut max_dd = 0.0_f64;
    let mut candle_idx: usize = 0;

    // Patch script once: replace ctx.* method calls with global fn names
    let patched = patch_script(&script_content);

    for (event_i, event) in events.iter().enumerate() {
        let ts_ms = event_ts(event);
        let ts = chrono::DateTime::from_timestamp_millis(ts_ms)
            .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .unwrap_or_else(|| ts_ms.to_string());

        match event {
            BacktestEvent::Candle { .. } => {
                let c = candle_from_event(event);
                {
                    let mut s = state.lock().unwrap();
                    s.candle_history.push(c.clone());
                }

                if has_on_candle {
                    run_on_candle(
                        &patched,
                        &state,
                        &c,
                        candle_idx,
                        fee_pct,
                        &ts,
                    );
                }
                candle_idx += 1;
            }
            BacktestEvent::Funding {
                venue,
                symbol,
                rate_apr,
                ..
            } => {
                {
                    let mut s = state.lock().unwrap();
                    s.latest_funding_apr
                        .insert((venue.clone(), symbol.clone()), *rate_apr);
                }

                if has_on_event {
                    run_on_event(
                        &patched,
                        &state,
                        event,
                        fee_pct,
                        &ts,
                        event_i,
                    );
                }
            }
        }

        // Portfolio snapshot: balance + mark-to-market of open positions
        let equity = {
            let s = state.lock().unwrap();
            let pos_value: f64 = s
                .positions
                .values()
                .map(|p| {
                    // For simplicity, mark to last known close price
                    let last_close = s
                        .candle_history
                        .last()
                        .map(|c| c.close)
                        .unwrap_or(p.entry_price);
                    if p.qty > 0.0 {
                        p.qty * last_close
                    } else {
                        // short: unrealised = (entry - last) * |qty|
                        (p.entry_price - last_close) * p.qty.abs()
                    }
                })
                .sum();
            s.balance + pos_value
        };
        portfolio_values.push(equity);
        if equity > peak {
            peak = equity;
        }
        let dd = if peak > 0.0 { (peak - equity) / peak * 100.0 } else { 0.0 };
        if dd > max_dd {
            max_dd = dd;
        }
    }

    // Close all remaining positions at last known price
    let last_price = state
        .lock()
        .unwrap()
        .candle_history
        .last()
        .map(|c| c.close)
        .unwrap_or(0.0);
    close_all_positions(&state, last_price, fee_pct);

    // Build metrics
    let mut s = state.lock().unwrap();
    let final_value = s.balance;
    let trades = std::mem::take(&mut s.trades);
    drop(s);

    let total_return_pct = (final_value / initial_balance - 1.0) * 100.0;
    let total_trades = trades.len() as u32;
    let wins = trades.iter().filter(|t| t.pnl > 0.0).count();
    let win_rate_pct = if total_trades > 0 {
        wins as f64 / total_trades as f64 * 100.0
    } else {
        0.0
    };

    let sharpe_ratio = if portfolio_values.len() > 1 {
        let returns: Vec<f64> = portfolio_values.windows(2).map(|w| w[1] / w[0] - 1.0).collect();
        let mean = returns.iter().sum::<f64>() / returns.len() as f64;
        let variance = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / returns.len() as f64;
        let std_dev = variance.sqrt();
        if std_dev > 0.0 {
            (mean / std_dev) * (252.0_f64).sqrt()
        } else {
            0.0
        }
    } else {
        0.0
    };

    let mut sorted_trades: Vec<&EventTrade> = trades.iter().collect();
    sorted_trades.sort_by(|a, b| a.pnl.partial_cmp(&b.pnl).unwrap_or(std::cmp::Ordering::Equal));
    let worst_trades: Vec<WorstTrade> = sorted_trades
        .iter()
        .take(5)
        .map(|t| WorstTrade {
            timestamp: t.timestamp.clone(),
            side: format!("{} {}", t.venue, t.side),
            price: t.price,
            pnl: t.pnl,
        })
        .collect();

    let mut running_balance = initial_balance;
    let all_trades: Vec<AllTrade> = trades
        .iter()
        .map(|t| {
            running_balance += t.pnl;
            AllTrade {
                timestamp: t.timestamp.clone(),
                side: format!("{} {}", t.venue, t.side),
                price: t.price,
                size: t.size,
                pnl: t.pnl,
                balance: running_balance,
                debug: None,
            }
        })
        .collect();

    let analysis = format!(
        "Event-driven backtest: {} events processed, {} trades. \
         Return {:.2}%, Sharpe {:.2}, Max DD {:.2}%, Win rate {:.1}%.",
        events.len(), total_trades, total_return_pct, sharpe_ratio, max_dd, win_rate_pct
    );

    Ok(BacktestMetrics {
        total_return_pct,
        sharpe_ratio,
        max_drawdown_pct: max_dd,
        win_rate_pct,
        total_trades,
        worst_trades,
        all_trades,
        analysis,
        avg_token_price: None,
        correct_direction_pct: None,
        break_even_win_rate: None,
        markets_tested: None,
        windows_with_real_price: None,
        windows_with_estimated_price: None,
        historical_data_coverage_pct: None,
        recommended_max_stake_usd: None,
        flat_debugs: vec![],
        ..Default::default()
    })
}

// ── Per-event execution ──────────────────────────────────────────────

fn run_on_candle(
    patched_script: &str,
    state: &Arc<Mutex<EventState>>,
    c: &Candle,
    idx: usize,
    fee_pct: f64,
    ts: &str,
) {
    let mut eng = Engine::new();
    eng.set_max_operations(500_000);
    eng.set_max_call_levels(64);

    register_event_fns(&mut eng, state, fee_pct, ts, idx);
    register_indicator_fns(&mut eng, state, idx);

    let (bal, _pos) = {
        let s = state.lock().unwrap();
        (s.balance, 0.0_f64)
    };

    let full = format!(
        r#"
{patched_script}

let ctx = #{{}};
ctx.close          = {close};
ctx.open           = {open};
ctx.high           = {high};
ctx.low            = {low};
ctx.volume         = {volume};
ctx.index          = {index};
ctx.position       = 0.0;
ctx.entry_price    = 0.0;
ctx.entry_index    = 0;
ctx.balance        = {balance};
ctx.open_positions = 0;
on_candle(ctx);
"#,
        patched_script = patched_script,
        close = c.close,
        open = c.open,
        high = c.high,
        low = c.low,
        volume = c.volume,
        index = idx,
        balance = bal,
    );

    if let Err(e) = eng.run(&full) {
        tracing::warn!("[EVENT-BACKTEST] on_candle error at idx {}: {}", idx, e);
    }
}

fn run_on_event(
    patched_script: &str,
    state: &Arc<Mutex<EventState>>,
    event: &BacktestEvent,
    fee_pct: f64,
    ts: &str,
    _event_i: usize,
) {
    let BacktestEvent::Funding {
        venue,
        symbol,
        rate,
        rate_apr,
        ..
    } = event
    else {
        return;
    };

    let mut eng = Engine::new();
    eng.set_max_operations(500_000);
    eng.set_max_call_levels(64);

    register_event_fns(&mut eng, state, fee_pct, ts, 0);
    register_indicator_fns(&mut eng, state, 0);

    let bal = { state.lock().unwrap().balance };

    let full = format!(
        r#"
{patched_script}

let ctx = #{{}};
ctx.balance = {balance};
ctx.index   = 0;

let event = #{{}};
event.kind       = "funding_cycle";
event.venue      = "{venue}";
event.symbol     = "{symbol}";
event.rate       = {rate};
event.rate_apr   = {rate_apr};

on_event(ctx, event);
"#,
        patched_script = patched_script,
        balance = bal,
        venue = venue,
        symbol = symbol,
        rate = rate,
        rate_apr = rate_apr,
    );

    if let Err(e) = eng.run(&full) {
        tracing::warn!(
            "[EVENT-BACKTEST] on_event error for {} {}: {}",
            venue,
            symbol,
            e
        );
    }
}

// ── Script patching ──────────────────────────────────────────────────

fn patch_script(script: &str) -> String {
    script
        .replace("ctx.rsi(", "rsi_impl(")
        .replace("ctx.ema(", "ema_impl(")
        .replace("ctx.atr(", "atr_impl(")
        .replace("ctx.sma(", "sma_impl(")
        .replace("ctx.macd_hist(", "macd_hist_impl(")
        .replace("ctx.close_at(", "close_at_impl(")
        .replace("ctx.high_at(", "high_at_impl(")
        .replace("ctx.low_at(", "low_at_impl(")
        .replace("ctx.volume_at(", "volume_at_impl(")
        .replace("ctx.set_stop_loss(", "set_stop_loss_impl(")
        .replace("ctx.set_take_profit(", "set_take_profit_impl(")
        .replace("ctx.bb_upper(", "bb_upper_impl(")
        .replace("ctx.bb_lower(", "bb_lower_impl(")
        .replace("ctx.bb_middle(", "bb_middle_impl(")
        .replace("ctx.bb_width(", "bb_width_impl(")
        .replace("ctx.stoch_k(", "stoch_k_impl(")
        .replace("ctx.vwap()", "vwap_impl()")
        .replace("ctx.stddev(", "stddev_impl(")
        .replace("ctx.realized_vol(", "realized_vol_impl(")
        .replace("ctx.kc_upper(", "kc_upper_impl(")
        .replace("ctx.kc_lower(", "kc_lower_impl(")
        .replace("ctx.short(", "short_impl(")
        .replace("ctx.buy(", "buy_impl(")
        .replace("ctx.sell(", "sell_impl(")
        .replace("ctx.set(", "set_impl(")
        .replace("ctx.get(", "get_impl(")
        .replace("ctx.log(", "log_impl(")
        // HyperLiquidity / event-driven extras
        .replace("ctx.funding_rate(", "funding_rate_impl(")
        .replace("ctx.funding_rate_apr(", "funding_rate_apr_impl(")
        .replace("ctx.position_at(", "position_at_impl(")
        .replace("ctx.regime()", "regime_impl()")
        .replace("ctx.basis_pct(", "basis_pct_impl(")
        .replace("ctx.short_at(", "short_at_impl(")
        .replace("ctx.buy_at(", "buy_at_impl(")
        .replace("ctx.close_at(", "close_at_impl(")
}

// ── Native fn registration ───────────────────────────────────────────

fn register_event_fns(
    eng: &mut Engine,
    state: &Arc<Mutex<EventState>>,
    fee_pct: f64,
    ts: &str,
    _idx: usize,
) {
    let ts = ts.to_string();

    // ctx.funding_rate_apr(venue, symbol) -> f64
    let s_fr = state.clone();
    eng.register_fn("funding_rate_apr_impl", move |venue: String, symbol: String| -> f64 {
        s_fr.lock()
            .unwrap()
            .latest_funding_apr
            .get(&(venue, symbol))
            .copied()
            .unwrap_or(0.0)
    });

    // ctx.funding_rate(venue, symbol) -> f64  (raw rate, not APR)
    // We don't store raw rate separately; return APR/365/3 as proxy for 8h rate
    let s_fr2 = state.clone();
    eng.register_fn("funding_rate_impl", move |venue: String, symbol: String| -> f64 {
        s_fr2
            .lock()
            .unwrap()
            .latest_funding_apr
            .get(&(venue, symbol))
            .copied()
            .unwrap_or(0.0)
            / (3.0 * 365.0)
    });

    // ctx.position_at(venue, symbol) -> f64 (qty, +long / -short)
    let s_pos = state.clone();
    eng.register_fn("position_at_impl", move |venue: String, symbol: String| -> f64 {
        s_pos
            .lock()
            .unwrap()
            .positions
            .get(&(venue, symbol))
            .map(|p| p.qty)
            .unwrap_or(0.0)
    });

    // ctx.buy_at(venue, symbol, size_usd) — open long
    let s_buy = state.clone();
    let buy_ts = ts.clone();
    let buy_fee = fee_pct;
    eng.register_fn("buy_at_impl", move |venue: String, symbol: String, size_usd: f64| {
        let mut s = s_buy.lock().unwrap();
        if s.balance <= 0.0 || size_usd <= 0.0 {
            return;
        }
        let amount = size_usd.min(s.balance);
        let fee_factor = 1.0 - buy_fee / 100.0;
        let last_close = s.candle_history.last().map(|c| c.close).unwrap_or(0.0);
        if last_close <= 0.0 {
            return;
        }
        let qty = (amount * fee_factor) / last_close;
        if qty <= 0.0 {
            return;
        }

        let key = (venue.clone(), symbol.clone());
        if let Some(pos) = s.positions.get_mut(&key) {
            // Add to existing position (weighted avg entry)
            let total_cost = pos.entry_price * pos.qty.abs() + last_close * qty;
            pos.entry_price = total_cost / (pos.qty.abs() + qty);
            pos.qty += qty;
            pos.size_usd += amount;
        } else {
            s.positions.insert(
                key,
                VenuePos {
                    qty,
                    entry_price: last_close,
                    size_usd: amount,
                },
            );
        }
        s.balance -= amount;
        s.trades.push(EventTrade {
            timestamp: buy_ts.clone(),
            side: "buy".into(),
            venue,
            symbol,
            price: last_close,
            size: qty,
            pnl: 0.0,
        });
    });

    // ctx.short_at(venue, symbol, size_usd) — open short
    let s_short = state.clone();
    let short_ts = ts.clone();
    let short_fee = fee_pct;
    eng.register_fn("short_at_impl", move |venue: String, symbol: String, size_usd: f64| {
        let mut s = s_short.lock().unwrap();
        if s.balance <= 0.0 || size_usd <= 0.0 {
            return;
        }
        let amount = size_usd.min(s.balance);
        let fee_factor = 1.0 - short_fee / 100.0;
        let last_close = s.candle_history.last().map(|c| c.close).unwrap_or(0.0);
        if last_close <= 0.0 {
            return;
        }
        let qty = (amount * fee_factor) / last_close;
        if qty <= 0.0 {
            return;
        }

        let key = (venue.clone(), symbol.clone());
        if let Some(pos) = s.positions.get_mut(&key) {
            let total_cost = pos.entry_price * pos.qty.abs() + last_close * qty;
            pos.entry_price = total_cost / (pos.qty.abs() + qty);
            pos.qty -= qty;
            pos.size_usd += amount;
        } else {
            s.positions.insert(
                key,
                VenuePos {
                    qty: -qty,
                    entry_price: last_close,
                    size_usd: amount,
                },
            );
        }
        s.balance -= amount;
        s.trades.push(EventTrade {
            timestamp: short_ts.clone(),
            side: "short".into(),
            venue,
            symbol,
            price: last_close,
            size: qty,
            pnl: 0.0,
        });
    });

    // ctx.close_at(venue, symbol) — close entire position
    let s_close = state.clone();
    let close_ts = ts.clone();
    let close_fee = fee_pct;
    eng.register_fn("close_at_impl", move |venue: String, symbol: String| {
        let mut s = s_close.lock().unwrap();
        let key = (venue.clone(), symbol.clone());
        let pos = match s.positions.remove(&key) {
            Some(p) => p,
            None => return,
        };
        let last_close = s.candle_history.last().map(|c| c.close).unwrap_or(0.0);
        if last_close <= 0.0 {
            return;
        }
        let fee_factor = 1.0 - close_fee / 100.0;
        let pnl = if pos.qty > 0.0 {
            // long: (exit - entry) * qty
            (last_close - pos.entry_price) * pos.qty * fee_factor
        } else {
            // short: (entry - exit) * |qty|
            (pos.entry_price - last_close) * pos.qty.abs() * fee_factor
        };
        s.balance += pos.size_usd + pnl;
        s.trades.push(EventTrade {
            timestamp: close_ts.clone(),
            side: "close".into(),
            venue,
            symbol,
            price: last_close,
            size: pos.qty.abs(),
            pnl,
        });
    });

    // ctx.basis_pct(venue, symbol) -> f64  (stub — requires spot+perp dual feed)
    eng.register_fn("basis_pct_impl", |_venue: String, _symbol: String| -> f64 { 0.0 });

    // ctx.regime() -> String
    let s_reg = state.clone();
    eng.register_fn("regime_impl", move || -> String {
        compute_regime(&s_reg.lock().unwrap().candle_history)
    });

    // ctx.set(key, val) / ctx.get(key, default)
    let s_set = state.clone();
    eng.register_fn("set_impl", move |key: String, val: f64| {
        s_set.lock().unwrap().kv.insert(key, val);
    });
    let s_set_i = state.clone();
    eng.register_fn("set_impl", move |key: String, val: i64| {
        s_set_i.lock().unwrap().kv.insert(key, val as f64);
    });
    let s_get = state.clone();
    eng.register_fn("get_impl", move |key: String, default: f64| -> f64 {
        s_get.lock().unwrap().kv.get(&key).copied().unwrap_or(default)
    });

    // ctx.log(msg)
    eng.register_fn("log_impl", move |msg: rhai::Dynamic| {
        tracing::info!("[STRATEGY-EVENT] {}", msg);
    });

    // Legacy buy/sell/short for scripts that also use on_candle
    let s_buy_legacy = state.clone();
    let buy_ts_leg = ts.clone();
    let buy_fee_leg = fee_pct;
    eng.register_fn("buy_impl", move |size: f64| {
        let mut s = s_buy_legacy.lock().unwrap();
        if s.balance <= 0.0 || size <= 0.0 {
            return;
        }
        let last_close = s.candle_history.last().map(|c| c.close).unwrap_or(0.0);
        if last_close <= 0.0 {
            return;
        }
        let amount = (s.balance * size.min(1.0)).min(s.balance);
        let fee_factor = 1.0 - buy_fee_leg / 100.0;
        let qty = (amount * fee_factor) / last_close;
        let key = ("default".into(), "default".into());
        if let Some(pos) = s.positions.get_mut(&key) {
            let total_cost = pos.entry_price * pos.qty.abs() + last_close * qty;
            pos.entry_price = total_cost / (pos.qty.abs() + qty);
            pos.qty += qty;
        } else {
            s.positions.insert(
                key,
                VenuePos {
                    qty,
                    entry_price: last_close,
                    size_usd: amount,
                },
            );
        }
        s.balance -= amount;
        s.trades.push(EventTrade {
            timestamp: buy_ts_leg.clone(),
            side: "buy".into(),
            venue: "default".into(),
            symbol: "default".into(),
            price: last_close,
            size: qty,
            pnl: 0.0,
        });
    });

    let s_sell_legacy = state.clone();
    let sell_ts_leg = ts.clone();
    let sell_fee_leg = fee_pct;
    eng.register_fn("sell_impl", move |_size: f64| {
        let mut s = s_sell_legacy.lock().unwrap();
        let key = ("default".into(), "default".into());
        let pos = match s.positions.remove(&key) {
            Some(p) => p,
            None => return,
        };
        let last_close = s.candle_history.last().map(|c| c.close).unwrap_or(0.0);
        if last_close <= 0.0 {
            return;
        }
        let fee_factor = 1.0 - sell_fee_leg / 100.0;
        let pnl = (last_close - pos.entry_price) * pos.qty * fee_factor;
        s.balance += pos.size_usd + pnl;
        s.trades.push(EventTrade {
            timestamp: sell_ts_leg.clone(),
            side: "sell".into(),
            venue: "default".into(),
            symbol: "default".into(),
            price: last_close,
            size: pos.qty,
            pnl,
        });
    });

    let s_short_legacy = state.clone();
    let short_ts_leg = ts.clone();
    let short_fee_leg = fee_pct;
    eng.register_fn("short_impl", move |_size: f64| {
        let mut s = s_short_legacy.lock().unwrap();
        if s.balance <= 0.0 {
            return;
        }
        let last_close = s.candle_history.last().map(|c| c.close).unwrap_or(0.0);
        if last_close <= 0.0 {
            return;
        }
        let amount = s.balance;
        let fee_factor = 1.0 - short_fee_leg / 100.0;
        let qty = (amount * fee_factor) / last_close;
        let key = ("default".into(), "default".into());
        s.positions.insert(
            key,
            VenuePos {
                qty: -qty,
                entry_price: last_close,
                size_usd: amount,
            },
        );
        s.balance -= amount;
        s.trades.push(EventTrade {
            timestamp: short_ts_leg.clone(),
            side: "short".into(),
            venue: "default".into(),
            symbol: "default".into(),
            price: last_close,
            size: qty,
            pnl: 0.0,
        });
    });

    // No-op stubs for stop loss / take profit in event-driven mode
    eng.register_fn("set_stop_loss_impl", |_price: f64| {});
    eng.register_fn("set_take_profit_impl", |_price: f64| {});
}

fn register_indicator_fns(eng: &mut Engine, state: &Arc<Mutex<EventState>>, idx: usize) {
    let s = state.clone();
    eng.register_fn("close_at_impl", move |i: i64| -> f64 {
        s.lock()
            .unwrap()
            .candle_history
            .get(i as usize)
            .map(|c| c.close)
            .unwrap_or(0.0)
    });

    let s = state.clone();
    eng.register_fn("volume_at_impl", move |i: i64| -> f64 {
        s.lock()
            .unwrap()
            .candle_history
            .get(i as usize)
            .map(|c| c.volume)
            .unwrap_or(0.0)
    });

    let s = state.clone();
    eng.register_fn("high_at_impl", move |i: i64| -> f64 {
        s.lock()
            .unwrap()
            .candle_history
            .get(i as usize)
            .map(|c| c.high)
            .unwrap_or(0.0)
    });

    let s = state.clone();
    eng.register_fn("low_at_impl", move |i: i64| -> f64 {
        s.lock()
            .unwrap()
            .candle_history
            .get(i as usize)
            .map(|c| c.low)
            .unwrap_or(0.0)
    });

    let s = state.clone();
    eng.register_fn("rsi_impl", move |period: i64| -> f64 {
        let hist = s.lock().unwrap().candle_history.clone();
        let period = period as usize;
        if idx < period || hist.len() < period + 1 {
            return 50.0;
        }
        let closes: Vec<f64> = hist.iter().map(|c| c.close).collect();
        let mut gain = 0.0_f64;
        let mut loss = 0.0_f64;
        for j in (idx - period + 1)..=idx {
            if j == 0 {
                continue;
            }
            let d = closes[j] - closes[j - 1];
            if d > 0.0 {
                gain += d;
            } else {
                loss += d.abs();
            }
        }
        gain /= period as f64;
        loss /= period as f64;
        if loss == 0.0 {
            100.0
        } else {
            100.0 - 100.0 / (1.0 + gain / loss)
        }
    });

    let s = state.clone();
    eng.register_fn("ema_impl", move |period: i64| -> f64 {
        let hist = s.lock().unwrap().candle_history.clone();
        let period = period as usize;
        if period == 0 || idx == 0 || hist.len() <= idx {
            return hist.get(idx).map(|c| c.close).unwrap_or(0.0);
        }
        let k = 2.0 / (period as f64 + 1.0);
        let start = idx.saturating_sub(period * 5);
        let closes: Vec<f64> = hist.iter().map(|c| c.close).collect();
        let mut e = closes[start];
        for j in (start + 1)..=idx {
            e = closes[j] * k + e * (1.0 - k);
        }
        e
    });

    let s = state.clone();
    eng.register_fn("sma_impl", move |period: i64| -> f64 {
        let hist = s.lock().unwrap().candle_history.clone();
        let period = period as usize;
        if period == 0 || hist.len() <= idx {
            return 0.0;
        }
        let start = if idx + 1 >= period {
            idx + 1 - period
        } else {
            0
        };
        let closes: Vec<f64> = hist.iter().map(|c| c.close).collect();
        closes[start..=idx].iter().sum::<f64>() / (idx - start + 1) as f64
    });

    let s = state.clone();
    eng.register_fn("atr_impl", move |period: i64| -> f64 {
        let hist = s.lock().unwrap().candle_history.clone();
        let period = (period.max(1)) as usize;
        if idx == 0 || hist.len() <= idx {
            return 0.0;
        }
        let start = idx.saturating_sub(period * 3);
        let mut tr_vals: Vec<f64> = Vec::new();
        for j in (start + 1)..=idx {
            let c = &hist[j];
            let p = &hist[j - 1];
            let tr = (c.high - c.low)
                .max((c.high - p.close).abs())
                .max((c.low - p.close).abs());
            tr_vals.push(tr);
        }
        if tr_vals.is_empty() {
            return 0.0;
        }
        if tr_vals.len() < period {
            return tr_vals.iter().sum::<f64>() / tr_vals.len() as f64;
        }
        let mut atr = tr_vals[..period].iter().sum::<f64>() / period as f64;
        for j in period..tr_vals.len() {
            atr = (atr * (period - 1) as f64 + tr_vals[j]) / period as f64;
        }
        atr
    });

    let s = state.clone();
    eng.register_fn("realized_vol_impl", move |period: i64| -> f64 {
        let hist = s.lock().unwrap().candle_history.clone();
        let period = period.max(2) as usize;
        if idx == 0 || hist.len() < 2 {
            return 0.0;
        }
        let start = if idx + 1 >= period {
            idx + 1 - period
        } else {
            0
        };
        if start >= idx {
            return 0.0;
        }
        let closes: Vec<f64> = hist.iter().map(|c| c.close).collect();
        let mut returns: Vec<f64> = Vec::with_capacity(idx - start);
        for j in (start + 1)..=idx {
            let r0 = closes[j - 1];
            let r1 = closes[j];
            if r0 > 0.0 && r1 > 0.0 {
                returns.push((r1 / r0).ln());
            }
        }
        if returns.is_empty() {
            return 0.0;
        }
        let mean = returns.iter().sum::<f64>() / returns.len() as f64;
        let var = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / returns.len() as f64;
        var.sqrt()
    });

    // bb_middle(period) - SMA
    let s = state.clone();
    eng.register_fn("bb_middle_impl", move |period: i64| -> f64 {
        let hist = s.lock().unwrap().candle_history.clone();
        let period = period as usize;
        if period == 0 || hist.len() <= idx { return 0.0; }
        let start = if idx + 1 >= period { idx + 1 - period } else { 0 };
        let closes: Vec<f64> = hist.iter().map(|c| c.close).collect();
        closes[start..=idx].iter().sum::<f64>() / (idx - start + 1) as f64
    });

    // bb_upper(period, mult) - SMA + mult * StdDev
    let s = state.clone();
    eng.register_fn("bb_upper_impl", move |period: i64, mult: f64| -> f64 {
        let hist = s.lock().unwrap().candle_history.clone();
        let period = period as usize;
        if period == 0 || hist.len() <= idx { return 0.0; }
        let start = if idx + 1 >= period { idx + 1 - period } else { 0 };
        let closes: Vec<f64> = hist.iter().map(|c| c.close).collect();
        let slice = &closes[start..=idx];
        let mean = slice.iter().sum::<f64>() / slice.len() as f64;
        let var = slice.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / slice.len() as f64;
        mean + mult * var.sqrt()
    });

    // bb_lower(period, mult) - SMA - mult * StdDev
    let s = state.clone();
    eng.register_fn("bb_lower_impl", move |period: i64, mult: f64| -> f64 {
        let hist = s.lock().unwrap().candle_history.clone();
        let period = period as usize;
        if period == 0 || hist.len() <= idx { return 0.0; }
        let start = if idx + 1 >= period { idx + 1 - period } else { 0 };
        let closes: Vec<f64> = hist.iter().map(|c| c.close).collect();
        let slice = &closes[start..=idx];
        let mean = slice.iter().sum::<f64>() / slice.len() as f64;
        let var = slice.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / slice.len() as f64;
        mean - mult * var.sqrt()
    });

    // bb_width(period, mult) - width as % of middle
    let s = state.clone();
    eng.register_fn("bb_width_impl", move |period: i64, mult: f64| -> f64 {
        let hist = s.lock().unwrap().candle_history.clone();
        let period = period as usize;
        if period == 0 || hist.len() <= idx { return 0.0; }
        let start = if idx + 1 >= period { idx + 1 - period } else { 0 };
        let closes: Vec<f64> = hist.iter().map(|c| c.close).collect();
        let slice = &closes[start..=idx];
        let mean = slice.iter().sum::<f64>() / slice.len() as f64;
        let var = slice.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / slice.len() as f64;
        let std = var.sqrt();
        if mean > 0.0 { (2.0 * mult * std) / mean * 100.0 } else { 0.0 }
    });

    // stoch_k(period)
    let s = state.clone();
    eng.register_fn("stoch_k_impl", move |period: i64| -> f64 {
        let hist = s.lock().unwrap().candle_history.clone();
        let period = period as usize;
        if period == 0 { return 50.0; }
        let start = if idx + 1 >= period { idx + 1 - period } else { 0 };
        let highest = hist[start..=idx].iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
        let lowest = hist[start..=idx].iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
        let close = hist.get(idx).map(|c| c.close).unwrap_or(0.0);
        if (highest - lowest).abs() < 1e-10 { return 50.0; }
        (close - lowest) / (highest - lowest) * 100.0
    });

    // vwap() - last 100 bars
    let s = state.clone();
    eng.register_fn("vwap_impl", move || -> f64 {
        let hist = s.lock().unwrap().candle_history.clone();
        let start = idx.saturating_sub(100);
        let mut sum_pv = 0.0_f64;
        let mut sum_v = 0.0_f64;
        for j in start..=idx {
            let v = hist.get(j).map(|c| c.volume).unwrap_or(0.0);
            let p = hist.get(j).map(|c| c.close).unwrap_or(0.0);
            sum_pv += p * v;
            sum_v += v;
        }
        if sum_v > 0.0 { sum_pv / sum_v } else { hist.get(idx).map(|c| c.close).unwrap_or(0.0) }
    });

    // stddev(period)
    let s = state.clone();
    eng.register_fn("stddev_impl", move |period: i64| -> f64 {
        let hist = s.lock().unwrap().candle_history.clone();
        let period = period as usize;
        if period == 0 || hist.len() <= idx { return 0.0; }
        let start = if idx + 1 >= period { idx + 1 - period } else { 0 };
        let closes: Vec<f64> = hist.iter().map(|c| c.close).collect();
        let slice = &closes[start..=idx];
        let mean = slice.iter().sum::<f64>() / slice.len() as f64;
        let var = slice.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / slice.len() as f64;
        var.sqrt()
    });

    // kc_upper(period, mult) - EMA + mult * ATR
    let s = state.clone();
    eng.register_fn("kc_upper_impl", move |period: i64, mult: f64| -> f64 {
        let hist = s.lock().unwrap().candle_history.clone();
        let period = period as usize;
        if period == 0 || idx == 0 || hist.len() <= idx { return 0.0; }
        // EMA
        let k = 2.0 / (period as f64 + 1.0);
        let ema_start = idx.saturating_sub(period * 5);
        let closes: Vec<f64> = hist.iter().map(|c| c.close).collect();
        let mut ema = closes[ema_start];
        for j in (ema_start + 1)..=idx {
            ema = closes[j] * k + ema * (1.0 - k);
        }
        // ATR
        let atr_start = idx.saturating_sub(period * 3);
        let mut tr_vals: Vec<f64> = Vec::new();
        for j in (atr_start + 1)..=idx {
            let c = hist[j].high;
            let l = hist[j].low;
            let p = hist[j - 1].close;
            let tr = (c - l).max((c - p).abs()).max((l - p).abs());
            tr_vals.push(tr);
        }
        let atr = if tr_vals.is_empty() {
            0.0
        } else if tr_vals.len() < period {
            tr_vals.iter().sum::<f64>() / tr_vals.len() as f64
        } else {
            let mut a = tr_vals[..period].iter().sum::<f64>() / period as f64;
            for j in period..tr_vals.len() {
                a = (a * (period - 1) as f64 + tr_vals[j]) / period as f64;
            }
            a
        };
        ema + mult * atr
    });

    // kc_lower(period, mult) - EMA - mult * ATR
    let s = state.clone();
    eng.register_fn("kc_lower_impl", move |period: i64, mult: f64| -> f64 {
        let hist = s.lock().unwrap().candle_history.clone();
        let period = period as usize;
        if period == 0 || idx == 0 || hist.len() <= idx { return 0.0; }
        let k = 2.0 / (period as f64 + 1.0);
        let ema_start = idx.saturating_sub(period * 5);
        let closes: Vec<f64> = hist.iter().map(|c| c.close).collect();
        let mut ema = closes[ema_start];
        for j in (ema_start + 1)..=idx {
            ema = closes[j] * k + ema * (1.0 - k);
        }
        let atr_start = idx.saturating_sub(period * 3);
        let mut tr_vals: Vec<f64> = Vec::new();
        for j in (atr_start + 1)..=idx {
            let c = hist[j].high;
            let l = hist[j].low;
            let p = hist[j - 1].close;
            let tr = (c - l).max((c - p).abs()).max((l - p).abs());
            tr_vals.push(tr);
        }
        let atr = if tr_vals.is_empty() {
            0.0
        } else if tr_vals.len() < period {
            tr_vals.iter().sum::<f64>() / tr_vals.len() as f64
        } else {
            let mut a = tr_vals[..period].iter().sum::<f64>() / period as f64;
            for j in period..tr_vals.len() {
                a = (a * (period - 1) as f64 + tr_vals[j]) / period as f64;
            }
            a
        };
        ema - mult * atr
    });

    // macd_hist remains a stub for now
    eng.register_fn("macd_hist_impl", |_fast: i64, _slow: i64, _signal: i64| -> f64 { 0.0 });
}

// ── Helpers ──────────────────────────────────────────────────────────

fn event_ts(event: &BacktestEvent) -> i64 {
    match event {
        BacktestEvent::Candle { ts_ms, .. } => *ts_ms,
        BacktestEvent::Funding { ts_ms, .. } => *ts_ms,
    }
}

fn candle_from_event(event: &BacktestEvent) -> Candle {
    match event {
        BacktestEvent::Candle {
            ts_ms,
            open,
            high,
            low,
            close,
            volume,
            ..
        } => Candle {
            open_time_ms: *ts_ms,
            open: *open,
            high: *high,
            low: *low,
            close: *close,
            volume: *volume,
        },
        BacktestEvent::Funding { .. } => Candle {
            open_time_ms: 0,
            open: 0.0,
            high: 0.0,
            low: 0.0,
            close: 0.0,
            volume: 0.0,
        },
    }
}

fn close_all_positions(state: &Arc<Mutex<EventState>>, last_price: f64, fee_pct: f64) {
    let mut s = state.lock().unwrap();
    let keys: Vec<(String, String)> = s.positions.keys().cloned().collect();
    for key in keys {
        let pos = s.positions.remove(&key).unwrap();
        if last_price <= 0.0 {
            continue;
        }
        let fee_factor = 1.0 - fee_pct / 100.0;
        let pnl = if pos.qty > 0.0 {
            (last_price - pos.entry_price) * pos.qty * fee_factor
        } else {
            (pos.entry_price - last_price) * pos.qty.abs() * fee_factor
        };
        s.balance += pos.size_usd + pnl;
        s.trades.push(EventTrade {
            timestamp: "final".into(),
            side: "close".into(),
            venue: key.0.clone(),
            symbol: key.1.clone(),
            price: last_price,
            size: pos.qty.abs(),
            pnl,
        });
    }
}

/// Simple regime detector based on recent candles.
fn compute_regime(candles: &[Candle]) -> String {
    if candles.len() < 20 {
        return "low_vol_chop".into();
    }
    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
    let n = closes.len();
    let recent = &closes[n - 20..];

    // Trend: slope of linear regression over last 20 closes
    let mean_x = 9.5_f64;
    let mean_y = recent.iter().sum::<f64>() / 20.0;
    let mut num = 0.0;
    let mut den = 0.0;
    for (i, y) in recent.iter().enumerate() {
        let x = i as f64;
        num += (x - mean_x) * (y - mean_y);
        den += (x - mean_x).powi(2);
    }
    let slope = if den > 0.0 { num / den } else { 0.0 };

    // Volatility: std dev of returns
    let mut rets: Vec<f64> = Vec::new();
    for i in 1..recent.len() {
        if recent[i - 1] > 0.0 {
            rets.push((recent[i] / recent[i - 1]).ln());
        }
    }
    let rv = if !rets.is_empty() {
        let m = rets.iter().sum::<f64>() / rets.len() as f64;
        (rets.iter().map(|r| (r - m).powi(2)).sum::<f64>() / rets.len() as f64).sqrt()
    } else {
        0.0
    };

    let trend_strength = slope.abs() / mean_y.abs();

    if rv < 0.0005 {
        "low_vol_chop".into()
    } else if trend_strength > 0.001 && rv > 0.001 {
        "trending".into()
    } else if rv > 0.0015 {
        "high_vol_chop".into()
    } else {
        "squeeze".into()
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_candles(n: usize) -> Vec<Candle> {
        (0..n)
            .map(|i| Candle {
                open_time_ms: i as i64 * 60_000,
                open: 100.0 + i as f64,
                high: 101.0 + i as f64,
                low: 99.0 + i as f64,
                close: 100.0 + i as f64,
                volume: 10.0,
            })
            .collect()
    }

    fn make_funding_events() -> Vec<BacktestEvent> {
        vec![
            BacktestEvent::Funding {
                ts_ms: 30_000,
                venue: "binance".into(),
                symbol: "BTCUSDT".into(),
                rate: 0.0001,
                rate_apr: 0.1095,
            },
            BacktestEvent::Funding {
                ts_ms: 30_000,
                venue: "hyperliquid".into(),
                symbol: "BTCUSDT".into(),
                rate: -0.0002,
                rate_apr: -0.219,
            },
        ]
    }

    #[test]
    fn event_backtest_smoke() {
        let mut events: Vec<BacktestEvent> = make_candles(10)
            .into_iter()
            .map(|c| BacktestEvent::Candle {
                ts_ms: c.open_time_ms,
                symbol: "BTCUSDT".into(),
                open: c.open,
                high: c.high,
                low: c.low,
                close: c.close,
                volume: c.volume,
            })
            .collect();
        events.extend(make_funding_events());
        events.sort_by_key(|e| match e {
            BacktestEvent::Candle { ts_ms, .. } => *ts_ms,
            BacktestEvent::Funding { ts_ms, .. } => *ts_ms,
        });

        let script = r#"
fn on_event(ctx, event) {
    if event.kind != "funding_cycle" { return; }
    let apr = ctx.funding_rate_apr(event.venue, event.symbol);
    ctx.set("last_apr", apr);
}
fn on_candle(ctx) {
    ctx.set("saw_candle", 1.0);
}
"#
        .to_string();

        let metrics = run_event_backtest(script, events, 10_000.0, 0.1).unwrap();
        assert!(metrics.total_trades == 0); // no trades in this dummy script
        assert!(metrics.total_return_pct == 0.0);
    }

    #[test]
    fn event_backtest_funding_rate_apr_lookup() {
        let mut events: Vec<BacktestEvent> = vec![BacktestEvent::Candle {
            ts_ms: 0,
            symbol: "BTCUSDT".into(),
            open: 100.0,
            high: 101.0,
            low: 99.0,
            close: 100.0,
            volume: 10.0,
        }];
        events.push(BacktestEvent::Funding {
            ts_ms: 30_000,
            venue: "binance".into(),
            symbol: "BTCUSDT".into(),
            rate: 0.0001,
            rate_apr: 0.1234,
        });
        events.push(BacktestEvent::Candle {
            ts_ms: 60_000,
            symbol: "BTCUSDT".into(),
            open: 100.0,
            high: 101.0,
            low: 99.0,
            close: 100.0,
            volume: 10.0,
        });

        let script = r#"
fn on_event(ctx, event) {
    if event.kind != "funding_cycle" { return; }
    let apr = ctx.funding_rate_apr("binance", "BTCUSDT");
    ctx.set("apr_binance", apr);
}
fn on_candle(ctx) {
    let apr = ctx.funding_rate_apr("binance", "BTCUSDT");
    if apr > 0.1 {
        ctx.buy_at("binance", "BTCUSDT", 1000.0);
    }
}
"#
        .to_string();

        let metrics = run_event_backtest(script, events, 10_000.0, 0.1).unwrap();
        // Should have executed a buy trade
        assert!(metrics.total_trades >= 1);
    }

    #[test]
    fn regime_detector_low_vol() {
        let candles: Vec<Candle> = (0..30)
            .map(|i| Candle {
                open_time_ms: i as i64 * 60_000,
                open: 100.0,
                high: 100.1,
                low: 99.9,
                close: 100.0,
                volume: 1.0,
            })
            .collect();
        assert_eq!(compute_regime(&candles), "low_vol_chop");
    }

    #[test]
    fn regime_detector_trending() {
        let candles: Vec<Candle> = (0..30)
            .map(|i| Candle {
                open_time_ms: i as i64 * 60_000,
                open: 100.0 + i as f64 * 0.5,
                high: 102.0 + i as f64 * 0.5 + (i % 3) as f64 * 0.3,
                low: 98.0 + i as f64 * 0.5 - (i % 3) as f64 * 0.3,
                close: 100.0 + i as f64 * 0.5 + (i % 5) as f64 * 0.1,
                volume: 1.0,
            })
            .collect();
        assert_eq!(compute_regime(&candles), "trending");
    }
}

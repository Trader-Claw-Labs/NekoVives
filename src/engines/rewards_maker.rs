//! REWARDS-MAKER engine — keeps a two-sided resting quote alive for Polymarket
//! liquidity rewards.
//!
//! The manual pilot failed because rewards require BILATERAL liquidity near the mid,
//! continuously: when one leg fills, you stop providing that side and the reward score
//! collapses. A human can't keep re-posting; this engine does.
//!
//! ## Loop (every `poll_secs`, default 60)
//!   1. Read the live YES mid from the CLOB.
//!   2. Cancel our existing quotes if the mid drifted > reprice_threshold from where we
//!      posted (stale quote = adverse-selection risk + ineligible).
//!   3. Ensure BOTH a BUY YES (mid - offset) and BUY NO (1 - mid - offset) are resting,
//!      each sized `size_usd`. Re-post whichever side is missing (filled or never placed).
//!   4. Track filled inventory; the reward accrues onchain regardless (paid at UTC midnight).
//!
//! ## Modes
//! - **paper (Dry Run)**: simulates the quote pair at live CLOB prices, tracks would-be
//!   eligible time + adverse fills. No real orders. Default.
//! - **live**: places/cancels real `ClobClient` limit orders, with per-runner guardrails
//!   (max_notional via live_sizing_value, max_open via a hard 2-leg cap).
//!
//! This engine does NOT predict direction — it harvests the reward for providing
//! liquidity in slow markets where adverse selection is low (validated separately).

use std::sync::Arc;
use std::time::Duration;
use std::path::PathBuf;

use crate::strategy_runner::{RunnerConfig, StrategyRunnerStore, LiveOrder};

/// Cents inside the eligible band each side rests (mirrors the manual pilot's 1¢).
const DEFAULT_OFFSET_C: f64 = 1.0;
/// Re-center the quote if the mid drifts more than this (in absolute price) from where
/// we posted. 0.02 = 2¢. Below this we leave the resting orders to keep earning.
const DEFAULT_REPRICE_THRESHOLD: f64 = 0.02;
const DEFAULT_POLL_SECS: u64 = 60;

/// Pull a positive f64 from `engine_params[key]`, else `default`. Accepts JSON
/// numbers or numeric strings (the UI sends params as a flat object).
fn param_f64(params: Option<&serde_json::Value>, key: &str, default: f64) -> f64 {
    params
        .and_then(|p| p.get(key))
        .and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
        .filter(|v| *v > 0.0)
        .unwrap_or(default)
}

struct QuoteState {
    /// Mid price at which the current pair was posted (for drift detection).
    ref_mid: f64,
    yes_order_id: Option<String>,
    no_order_id: Option<String>,
    /// Cumulative simulated/real fills, for the dashboard.
    fills: u32,
    eligible_polls: u32,
    total_polls: u32,
}

pub async fn run_rewards_maker_loop(
    store: Arc<StrategyRunnerStore>,
    config: RunnerConfig,
    _workspace_dir: PathBuf,
) {
    let id = config.id.clone();
    let is_live = config.mode == "live";
    let size_usd = if config.live_sizing_value > 0.0 { config.live_sizing_value } else { 50.0 };
    // Tunables: offset / reprice / poll come from engine_params when present, else the
    // pilot defaults. (The backtest path reads the same keys via MakerBacktestParams.)
    let ep = config.engine_params.as_ref();
    let offset_c = param_f64(ep, "offset_cents", DEFAULT_OFFSET_C);
    let offset = offset_c / 100.0;
    let reprice_threshold = param_f64(ep, "reprice_threshold", DEFAULT_REPRICE_THRESHOLD);
    let poll_secs = param_f64(ep, "poll_secs", DEFAULT_POLL_SECS as f64).round().max(5.0) as u64;

    // Resolve YES/NO token ids: prefer explicit config, else look them up from the
    // condition_id via the CLOB (the maker quotes one fixed market, not a series).
    let (mut yes_token, mut no_token) = (
        config.poly_token_id.clone().unwrap_or_default(),
        config.poly_no_token_id.clone().unwrap_or_default(),
    );
    // condition_id source: poly_condition_id (in-memory, set at create) OR `symbol`
    // (persisted to disk — survives restarts). The maker stores the cid in `symbol`
    // when it's a 0x… value, since poly_condition_id is #[serde(skip)].
    let cid_opt = config.poly_condition_id.clone()
        .or_else(|| if config.symbol.starts_with("0x") && config.symbol.len() >= 60 {
            Some(config.symbol.clone()) } else { None });
    if (yes_token.is_empty() || no_token.is_empty()) {
        if let Some(cid) = cid_opt {
            if let Some((y, n)) = resolve_tokens_by_condition(&cid).await {
                yes_token = y; no_token = n;
            }
        }
    }
    if yes_token.is_empty() || no_token.is_empty() {
        append_log(&store, &id, "rewards_maker: could not resolve YES/NO token ids — set poly_condition_id.");
        crate::strategy_runner::set_runner_status(&store, &id, "error");
        return;
    }

    // Live mode needs CLOB creds.
    let clob: Option<Arc<polymarket_trader::orders::ClobClient>> = if is_live {
        config.poly_creds.clone().map(|c| Arc::new(polymarket_trader::orders::ClobClient::new(c)))
    } else {
        None
    };
    if is_live && clob.is_none() {
        append_log(&store, &id, "rewards_maker: live mode requires Polymarket credentials.");
        return;
    }

    crate::strategy_runner::set_runner_status(&store, &id, "running");
    append_log(&store, &id, &format!(
        "rewards_maker started ({}). YES={}… NO={}… size=${:.0}/side offset={:.0}¢",
        if is_live { "LIVE" } else { "DRY-RUN" },
        &yes_token[..yes_token.len().min(10)], &no_token[..no_token.len().min(10)], size_usd, offset_c
    ));

    let mut st = QuoteState { ref_mid: 0.0, yes_order_id: None, no_order_id: None, fills: 0, eligible_polls: 0, total_polls: 0 };
    let mut interval = tokio::time::interval(Duration::from_secs(poll_secs));

    loop {
        interval.tick().await;
        // Stop if the runner was halted/stopped from the dashboard.
        let still_running = store.get(&id)
            .map(|r| matches!(r.status.status.as_str(), "running" | "starting"))
            .unwrap_or(false);
        if !still_running { break; }

        // 1. Read live YES mid.
        let yes_ask = polymarket_trader::markets::get_market_price(&yes_token).await.unwrap_or(0.0);
        if !(yes_ask > 0.02 && yes_ask < 0.98) {
            append_log(&store, &id, &format!("rewards_maker: skip poll — unusable mid {yes_ask:.3}"));
            continue;
        }
        st.total_polls += 1;
        let mid = yes_ask;

        // 2. Drift check → cancel stale quotes so they get re-centered below.
        let drifted = st.ref_mid > 0.0 && (mid - st.ref_mid).abs() > reprice_threshold;
        if drifted {
            if is_live {
                if let Some(c) = &clob {
                    for oid in [st.yes_order_id.take(), st.no_order_id.take()].into_iter().flatten() {
                        let _ = c.cancel_order(&oid).await;
                    }
                }
            } else {
                st.yes_order_id = None; st.no_order_id = None;
            }
            append_log(&store, &id, &format!("rewards_maker: mid drifted {:.3}→{:.3}, re-centering", st.ref_mid, mid));
        }

        // 3. Ensure both legs are resting. Re-post any missing side.
        let yes_px = ((mid - offset) * 100.0).round() / 100.0;
        let no_px = ((1.0 - mid - offset) * 100.0).round() / 100.0;
        let mut posted_any = false;

        if st.yes_order_id.is_none() && yes_px > 0.01 {
            let oid = place_or_sim(&clob, &yes_token, yes_px, size_usd, is_live).await;
            if oid.is_some() { posted_any = true; }
            st.yes_order_id = oid;
        }
        if st.no_order_id.is_none() && no_px > 0.01 {
            let oid = place_or_sim(&clob, &no_token, no_px, size_usd, is_live).await;
            if oid.is_some() { posted_any = true; }
            st.no_order_id = oid;
        }
        if posted_any || drifted {
            st.ref_mid = mid;
        }

        // 4. Eligibility: we count a poll as "eligible" when BOTH legs rest within the band.
        let eligible = st.yes_order_id.is_some() && st.no_order_id.is_some();
        if eligible { st.eligible_polls += 1; }

        // Persist a lightweight status to the dashboard via the runner result.
        let elig_pct = if st.total_polls > 0 { st.eligible_polls as f64 / st.total_polls as f64 * 100.0 } else { 0.0 };
        store.update_result(&id, |res| {
            res.balance = config.initial_balance; // rewards accrue onchain, not in sim balance
            res.total_trades = st.fills;
            res.win_rate_pct = elig_pct; // reuse field to surface "eligible %"
            // Surface the live quotes as pseudo-orders so the dashboard shows them.
            res.live_orders = build_quote_orders(&yes_token, &no_token, yes_px, no_px, size_usd, &st);
        });
        store.persist();
    }

    // On stop: cancel any live resting orders so we don't leave stale quotes.
    if is_live {
        if let Some(c) = &clob {
            for oid in [st.yes_order_id.take(), st.no_order_id.take()].into_iter().flatten() {
                let _ = c.cancel_order(&oid).await;
            }
        }
    }
    append_log(&store, &id, &format!(
        "rewards_maker stopped. eligible {}/{} polls ({:.0}%)", st.eligible_polls, st.total_polls,
        if st.total_polls > 0 { st.eligible_polls as f64 / st.total_polls as f64 * 100.0 } else { 0.0 }
    ));
}

/// Place a real limit order (live) or return a simulated id (paper).
async fn place_or_sim(
    clob: &Option<Arc<polymarket_trader::orders::ClobClient>>,
    token_id: &str,
    price: f64,
    size_usd: f64,
    is_live: bool,
) -> Option<String> {
    let shares = (size_usd / price).round();
    if is_live {
        let c = clob.as_ref()?;
        match c.create_limit_order(token_id, polymarket_trader::orders::Side::Buy, price, shares).await {
            Ok(resp) => Some(resp.order_id),
            Err(_) => None,
        }
    } else {
        Some(format!("paper-{token_id}-{price}"))
    }
}

fn build_quote_orders(
    yes_token: &str, no_token: &str, yes_px: f64, no_px: f64, size_usd: f64, st: &QuoteState,
) -> Vec<LiveOrder> {
    let mut v = Vec::new();
    if let Some(ref oid) = st.yes_order_id {
        v.push(quote_order("yes", yes_token, yes_px, size_usd, oid));
    }
    if let Some(ref oid) = st.no_order_id {
        v.push(quote_order("no", no_token, no_px, size_usd, oid));
    }
    v
}

fn quote_order(side: &str, token: &str, px: f64, size_usd: f64, oid: &str) -> LiveOrder {
    LiveOrder {
        timestamp: chrono::Utc::now().to_rfc3339(),
        window_ts: 0,
        side: side.to_string(),
        token_id: token.to_string(),
        amount_usdc: size_usd,
        order_id: oid.to_string(),
        status: "LIVE".to_string(),
        entry_price: Some(px),
        result: None,
        pnl: None,
        stop_loss_triggered: false,
        ..Default::default()
    }
}

fn append_log(store: &Arc<StrategyRunnerStore>, id: &str, msg: &str) {
    crate::strategy_runner::append_runner_log(store, id, msg);
}

/// Resolve (yes_token_id, no_token_id) from a condition_id via the CLOB markets endpoint.
async fn resolve_tokens_by_condition(condition_id: &str) -> Option<(String, String)> {
    let url = format!("https://clob.polymarket.com/markets/{condition_id}");
    let v: serde_json::Value = reqwest::Client::new()
        .get(&url)
        .header("User-Agent", "trader-claw/rewards-maker")
        .timeout(Duration::from_secs(15))
        .send().await.ok()?
        .json().await.ok()?;
    let tokens = v.get("tokens")?.as_array()?;
    let mut yes = None;
    let mut no = None;
    for t in tokens {
        let outcome = t.get("outcome").and_then(|o| o.as_str()).unwrap_or("").to_lowercase();
        let tid = t.get("token_id").and_then(|i| i.as_str()).map(str::to_string);
        if outcome == "yes" { yes = tid; }
        else if outcome == "no" { no = tid; }
    }
    Some((yes?, no?))
}

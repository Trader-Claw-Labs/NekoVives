//! REWARDS-ORCHESTRATOR engine — autonomous liquidity-rewards farming across a POOL
//! of slow reward markets.
//!
//! This is the multi-market sibling of `rewards_maker`. Where rewards_maker quotes ONE
//! fixed condition_id, the orchestrator owns the whole pilot:
//!   1. Every `poll_secs` it re-scans the reward market list (`scan_reward_markets`).
//!   2. It auto-selects the top-N markets that clear `min_safety` and are NOT toxic.
//!   3. For each selected market it keeps a two-sided resting quote alive (BUY YES +
//!      BUY NO at mid ± offset), re-posting whichever side fills.
//!   4. If a market we hold turns toxic (or drops below min_safety, or expires), it
//!      CLOSES that market's quotes and drops it from the pool.
//!   5. Freed capacity rotates into the next-best fresh market on the following poll.
//!
//! Capital model: `size_usd` per side is `total_capital / (max_markets * 2)` unless an
//! explicit `size_usd` is given in engine_params. Rewards accrue onchain (paid at UTC
//! midnight) — they are NOT reflected in the sim balance; the dashboard surfaces the
//! number of active markets + eligible quote count instead.
//!
//! Modes: paper simulates quotes at live CLOB prices; live places/cancels real orders.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use std::path::PathBuf;

use crate::strategy_runner::{RunnerConfig, StrategyRunnerStore, LiveOrder};

const DEFAULT_OFFSET_C: f64 = 1.0;
const DEFAULT_REPRICE_THRESHOLD: f64 = 0.02;
const DEFAULT_POLL_SECS: u64 = 60;
const DEFAULT_MAX_MARKETS: usize = 3;
const DEFAULT_MIN_SAFETY: &str = "high";

fn param_f64(params: Option<&serde_json::Value>, key: &str, default: f64) -> f64 {
    params
        .and_then(|p| p.get(key))
        .and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
        .filter(|v| *v > 0.0)
        .unwrap_or(default)
}

fn param_str(params: Option<&serde_json::Value>, key: &str, default: &str) -> String {
    params
        .and_then(|p| p.get(key))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default.to_string())
}

/// Rank of a safety label — higher is safer. Used to gate the pool by `min_safety`.
fn safety_rank(s: &str) -> u8 {
    match s {
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0, // toxic / expiring
    }
}

/// Live quote state for one market in the pool.
struct MarketQuote {
    condition_id: String,
    question: String,
    yes_token: String,
    no_token: String,
    ref_mid: f64,
    yes_order_id: Option<String>,
    no_order_id: Option<String>,
}

pub async fn run_rewards_orchestrator_loop(
    store: Arc<StrategyRunnerStore>,
    config: RunnerConfig,
    _workspace_dir: PathBuf,
) {
    let id = config.id.clone();
    let is_live = config.mode == "live";
    let ep = config.engine_params.as_ref();

    let offset = param_f64(ep, "offset_cents", DEFAULT_OFFSET_C) / 100.0;
    let reprice_threshold = param_f64(ep, "reprice_threshold", DEFAULT_REPRICE_THRESHOLD);
    let poll_secs = param_f64(ep, "poll_secs", DEFAULT_POLL_SECS as f64).round().max(10.0) as u64;
    let max_markets = param_f64(ep, "max_markets", DEFAULT_MAX_MARKETS as f64).round().max(1.0) as usize;
    let min_safety = param_str(ep, "min_safety", DEFAULT_MIN_SAFETY);
    let min_safety_rank = safety_rank(&min_safety);

    // Per-side size: explicit override, else split the capital across both legs of every
    // market slot so the worst case (all legs filled) stays within the assigned capital.
    let size_usd = {
        let explicit = param_f64(ep, "size_usd", 0.0);
        if explicit > 0.0 {
            explicit
        } else {
            (config.initial_balance / (max_markets as f64 * 2.0)).max(1.0)
        }
    };

    let clob: Option<Arc<polymarket_trader::orders::ClobClient>> = if is_live {
        config.poly_creds.clone().map(|c| Arc::new(polymarket_trader::orders::ClobClient::new(c)))
    } else {
        None
    };
    if is_live && clob.is_none() {
        append_log(&store, &id, "rewards_orchestrator: live mode requires Polymarket credentials.");
        crate::strategy_runner::set_runner_status(&store, &id, "error");
        return;
    }

    crate::strategy_runner::set_runner_status(&store, &id, "running");
    append_log(&store, &id, &format!(
        "rewards_orchestrator started ({}). pool={} markets · min_safety={} · ${:.0}/side · offset={:.0}¢ · poll={}s",
        if is_live { "LIVE" } else { "DRY-RUN" }, max_markets, min_safety, size_usd, offset * 100.0, poll_secs
    ));

    // Active pool keyed by condition_id.
    let mut pool: HashMap<String, MarketQuote> = HashMap::new();
    let mut total_polls: u32 = 0;
    let mut eligible_polls: u32 = 0; // polls where every active market had both legs resting
    let mut fills: u32 = 0;

    let mut interval = tokio::time::interval(Duration::from_secs(poll_secs));

    loop {
        interval.tick().await;
        let still_running = store.get(&id)
            .map(|r| matches!(r.status.status.as_str(), "running" | "starting"))
            .unwrap_or(false);
        if !still_running { break; }
        total_polls += 1;

        // 1. Re-scan the reward market universe. On failure keep the existing pool alive.
        let markets = match market_analyzer::rewards::scan_reward_markets(3).await {
            Ok(m) => m,
            Err(e) => { append_log(&store, &id, &format!("rewards_orchestrator: scan failed ({e}); keeping current pool")); continue; }
        };

        // 2. Build the safe candidate set (non-toxic, clears min_safety, has both tokens),
        //    already score-sorted by the scanner.
        let safe: Vec<&market_analyzer::rewards::RewardMarket> = markets.iter()
            .filter(|m| !m.is_toxic
                && safety_rank(&m.safety) >= min_safety_rank
                && m.yes_token_id.is_some() && m.no_token_id.is_some())
            .collect();
        let safe_ids: std::collections::HashSet<&str> = safe.iter().map(|m| m.condition_id.as_str()).collect();

        // 3. Evict any held market that is no longer safe (turned toxic / dropped below
        //    min_safety / expired). Close its quotes first.
        let evict: Vec<String> = pool.keys()
            .filter(|cid| !safe_ids.contains(cid.as_str()))
            .cloned()
            .collect();
        for cid in evict {
            if let Some(mut q) = pool.remove(&cid) {
                close_quotes(&clob, &mut q, is_live).await;
                append_log(&store, &id, &format!("rewards_orchestrator: market turned unsafe — closed & dropped \"{}\"", trunc(&q.question, 60)));
            }
        }

        // 4. Backfill the pool up to max_markets from the top of the safe list.
        for m in safe.iter() {
            if pool.len() >= max_markets { break; }
            if pool.contains_key(&m.condition_id) { continue; }
            let (Some(yes), Some(no)) = (m.yes_token_id.clone(), m.no_token_id.clone()) else { continue };
            pool.insert(m.condition_id.clone(), MarketQuote {
                condition_id: m.condition_id.clone(),
                question: m.question.clone(),
                yes_token: yes, no_token: no,
                ref_mid: 0.0, yes_order_id: None, no_order_id: None,
            });
            append_log(&store, &id, &format!("rewards_orchestrator: added \"{}\" (safety={}, ${:.0}/day)", trunc(&m.question, 60), m.safety, m.daily_rate));
        }

        // 5. Maintain a two-sided quote on every active market.
        let mut all_eligible = !pool.is_empty();
        let mut live_orders: Vec<LiveOrder> = Vec::new();
        for q in pool.values_mut() {
            let yes_ask = polymarket_trader::markets::get_market_price(&q.yes_token).await.unwrap_or(0.0);
            if !(yes_ask > 0.02 && yes_ask < 0.98) {
                all_eligible = false;
                continue;
            }
            let mid = yes_ask;

            // Re-center on drift.
            if q.ref_mid > 0.0 && (mid - q.ref_mid).abs() > reprice_threshold {
                close_quotes(&clob, q, is_live).await;
            }

            let yes_px = ((mid - offset) * 100.0).round() / 100.0;
            let no_px = ((1.0 - mid - offset) * 100.0).round() / 100.0;
            let mut posted_any = false;
            if q.yes_order_id.is_none() && yes_px > 0.01 {
                let oid = place_or_sim(&clob, &q.yes_token, yes_px, size_usd, is_live).await;
                if oid.is_some() { posted_any = true; }
                q.yes_order_id = oid;
            }
            if q.no_order_id.is_none() && no_px > 0.01 {
                let oid = place_or_sim(&clob, &q.no_token, no_px, size_usd, is_live).await;
                if oid.is_some() { posted_any = true; }
                q.no_order_id = oid;
            }
            if posted_any { q.ref_mid = mid; }

            let eligible = q.yes_order_id.is_some() && q.no_order_id.is_some();
            if !eligible { all_eligible = false; }
            if let Some(ref oid) = q.yes_order_id { live_orders.push(quote_order("yes", &q.yes_token, yes_px, size_usd, oid, &q.condition_id)); }
            if let Some(ref oid) = q.no_order_id { live_orders.push(quote_order("no", &q.no_token, no_px, size_usd, oid, &q.condition_id)); }
        }
        if all_eligible { eligible_polls += 1; }

        let elig_pct = if total_polls > 0 { eligible_polls as f64 / total_polls as f64 * 100.0 } else { 0.0 };
        let n_markets = pool.len();
        store.update_result(&id, |res| {
            res.balance = config.initial_balance; // rewards accrue onchain, not in sim balance
            res.total_trades = fills;
            res.win_rate_pct = elig_pct; // reuse field: "eligible %"
            // Surface pool state for the dashboard (no dedicated field exists).
            res.live_kv_state.insert("active_markets".to_string(), n_markets as f64);
            res.live_kv_state.insert("eligible_pct".to_string(), elig_pct);
            res.live_orders = live_orders.clone();
        });
        store.persist();
    }

    // On stop: close every live quote so we don't leave stale orders.
    for (_, mut q) in pool.drain() {
        close_quotes(&clob, &mut q, is_live).await;
    }
    append_log(&store, &id, &format!(
        "rewards_orchestrator stopped. eligible {}/{} polls ({:.0}%)", eligible_polls, total_polls,
        if total_polls > 0 { eligible_polls as f64 / total_polls as f64 * 100.0 } else { 0.0 }
    ));
}

/// Cancel (live) or clear (paper) both legs of a market's quote.
async fn close_quotes(
    clob: &Option<Arc<polymarket_trader::orders::ClobClient>>,
    q: &mut MarketQuote,
    is_live: bool,
) {
    if is_live {
        if let Some(c) = clob {
            for oid in [q.yes_order_id.take(), q.no_order_id.take()].into_iter().flatten() {
                let _ = c.cancel_order(&oid).await;
            }
        }
    }
    q.yes_order_id = None;
    q.no_order_id = None;
    q.ref_mid = 0.0;
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

fn quote_order(side: &str, token: &str, px: f64, size_usd: f64, oid: &str, cid: &str) -> LiveOrder {
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
        condition_id: Some(cid.to_string()),
        ..Default::default()
    }
}

fn append_log(store: &Arc<StrategyRunnerStore>, id: &str, msg: &str) {
    crate::strategy_runner::append_runner_log(store, id, msg);
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { format!("{}…", s.chars().take(n).collect::<String>()) }
}

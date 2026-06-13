//! Handler for /strat Telegram commands.
//!
//! Provides read/control access to running strategy engines from Telegram.
//! Read commands (list, status, pnl) are open to all authenticated users.
//! Control commands (stop) require `is_admin`.

use std::sync::Arc;
use crate::strategy_runner::StrategyRunnerStore;

// ── Result type ───────────────────────────────────────────────────────────────

/// Result of handling a /strat command — the formatted Markdown reply.
pub struct StratCommandResult {
    pub message: String,
    pub is_error: bool,
}

impl StratCommandResult {
    fn ok(message: impl Into<String>) -> Self {
        Self { message: message.into(), is_error: false }
    }

    fn err(message: impl Into<String>) -> Self {
        Self { message: message.into(), is_error: true }
    }
}

// ── Global store accessor ─────────────────────────────────────────────────────

static RUNNER_STORE: std::sync::OnceLock<Arc<StrategyRunnerStore>> = std::sync::OnceLock::new();

/// Called once at app startup (gateway init) to register the runner store.
/// Subsequent calls are silently ignored (OnceLock guarantees single init).
pub fn init_runner_store(store: Arc<StrategyRunnerStore>) {
    let _ = RUNNER_STORE.set(store);
}

/// Get a reference to the global runner store, if initialised.
pub fn runner_store() -> Option<&'static Arc<StrategyRunnerStore>> {
    RUNNER_STORE.get()
}

// ── Command handler ───────────────────────────────────────────────────────────

/// Parse and handle a `/strat` command.
///
/// - `text`: full message text (e.g. `"/strat status my-runner"`)
/// - `is_admin`: write/control commands (stop) require this to be `true`
/// - `store`: the live strategy runner store
pub fn handle_strat_command(
    text: &str,
    is_admin: bool,
    store: &Arc<StrategyRunnerStore>,
) -> StratCommandResult {
    let parts: Vec<&str> = text.split_whitespace().collect();
    let sub = parts.get(1).copied().unwrap_or("");

    match sub {
        "list" => handle_list(store),
        "status" => {
            let id = match parts.get(2) {
                Some(id) => *id,
                None => return StratCommandResult::err("Usage: `/strat status <id>`"),
            };
            handle_status(id, store)
        }
        "pnl" => handle_pnl(store),
        "stop" => {
            if !is_admin {
                return StratCommandResult::err("Permission denied. Only admins can stop strategies.");
            }
            let id = match parts.get(2) {
                Some(id) => *id,
                None => return StratCommandResult::err("Usage: `/strat stop <id>`"),
            };
            handle_stop(id, store)
        }
        _ => handle_help(),
    }
}

// ── Subcommand handlers ───────────────────────────────────────────────────────

fn handle_list(store: &Arc<StrategyRunnerStore>) -> StratCommandResult {
    let runners = store.list();
    if runners.is_empty() {
        return StratCommandResult::ok("*Strategies*\n\nNo strategies configured yet.");
    }

    let mut msg = String::from("*Running Strategies*\n\n");
    for r in &runners {
        let status_icon = match r.status.status.as_str() {
            "running"  => "🟢",
            "stopped"  => "🔴",
            "error"    => "🔴",
            "starting" => "🟡",
            _          => "⚪",
        };
        let balance = r.result.as_ref().map(|res| res.balance).unwrap_or(r.config.initial_balance);
        let pnl_pct = r.result.as_ref().map(|res| res.total_return_pct).unwrap_or(0.0);
        let kind    = r.config.kind.as_deref().unwrap_or("rhai_candle");
        let sign    = if pnl_pct >= 0.0 { "+" } else { "" };

        msg.push_str(&format!(
            "{status_icon} *{}* (`{kind}`)\n   💰 ${balance:.2}  {sign}{pnl_pct:.2}%\n\n",
            r.config.name,
        ));
    }
    msg.push_str(&format!("_{} strategy(ies) total_", runners.len()));
    StratCommandResult::ok(msg)
}

fn handle_status(id: &str, store: &Arc<StrategyRunnerStore>) -> StratCommandResult {
    let runner = match store.get(id) {
        Some(r) => r,
        None => return StratCommandResult::err(format!("Strategy `{id}` not found.")),
    };

    let cfg  = &runner.config;
    let stat = &runner.status;
    let res  = runner.result.as_ref();

    let balance   = res.map(|r| r.balance).unwrap_or(cfg.initial_balance);
    let pnl_pct   = res.map(|r| r.total_return_pct).unwrap_or(0.0);
    let win_rate  = res.map(|r| r.win_rate_pct).unwrap_or(0.0);
    let trades    = res.map(|r| r.total_trades).unwrap_or(0);
    let sharpe    = res.map(|r| r.sharpe_ratio).unwrap_or(0.0);
    let signal    = res.map(|r| r.last_signal.as_str()).unwrap_or("—").to_string();
    let analysis  = res.map(|r| r.analysis.as_str()).unwrap_or("—").to_string();
    let kind      = cfg.kind.as_deref().unwrap_or("rhai_candle");
    let sign      = if pnl_pct >= 0.0 { "+" } else { "" };

    let msg = format!(
        "*Strategy: {}*\n\
         ID: `{}`\n\
         Kind: `{kind}`\n\
         Markets: `{}`\n\
         Status: `{}`\n\
         Started: {}\n\n\
         💰 Balance: ${balance:.2}\n\
         📈 P&L: {sign}{pnl_pct:.2}%\n\
         🎯 Win rate: {win_rate:.1}%\n\
         📊 Trades: {trades}\n\
         ⚡ Sharpe: {sharpe:.2}\n\
         🔔 Last signal: {signal}\n\n\
         📝 {analysis}",
        cfg.name, cfg.id, cfg.symbol,
        stat.status, stat.started_at,
    );
    StratCommandResult::ok(msg)
}

fn handle_pnl(store: &Arc<StrategyRunnerStore>) -> StratCommandResult {
    let runners = store.list();
    if runners.is_empty() {
        return StratCommandResult::ok("*P&L Summary*\n\nNo strategies to report.");
    }

    let mut total_invested = 0.0_f64;
    let mut total_balance  = 0.0_f64;
    let mut best_pnl  = f64::NEG_INFINITY;
    let mut worst_pnl = f64::INFINITY;
    let mut best_name  = String::new();
    let mut worst_name = String::new();
    let mut total_trades = 0u32;

    for r in &runners {
        let init = r.config.initial_balance;
        let bal  = r.result.as_ref().map(|res| res.balance).unwrap_or(init);
        let pnl  = r.result.as_ref().map(|res| res.total_return_pct).unwrap_or(0.0);
        let tr   = r.result.as_ref().map(|res| res.total_trades).unwrap_or(0);

        total_invested += init;
        total_balance  += bal;
        total_trades   += tr;

        if pnl > best_pnl  { best_pnl  = pnl; best_name  = r.config.name.clone(); }
        if pnl < worst_pnl { worst_pnl = pnl; worst_name = r.config.name.clone(); }
    }

    let agg_pnl  = if total_invested > 0.0 { (total_balance - total_invested) / total_invested * 100.0 } else { 0.0 };
    let agg_sign = if agg_pnl >= 0.0 { "+" } else { "" };

    let msg = format!(
        "*P&L Summary* ({} strategies)\n\n\
         💰 Total invested: ${total_invested:.2}\n\
         💵 Total balance:  ${total_balance:.2}\n\
         📈 Aggregate P&L:  {agg_sign}{agg_pnl:.2}%\n\
         📊 Total trades:   {total_trades}\n\n\
         🏆 Best:  {} ({}{best_pnl:.2}%)\n\
         📉 Worst: {} ({}{worst_pnl:.2}%)",
        runners.len(),
        best_name,  if best_pnl  >= 0.0 { "+" } else { "" },
        worst_name, if worst_pnl >= 0.0 { "+" } else { "" },
    );
    StratCommandResult::ok(msg)
}

fn handle_stop(id: &str, store: &Arc<StrategyRunnerStore>) -> StratCommandResult {
    if store.stop(id) {
        StratCommandResult::ok(format!("Strategy `{id}` stopped successfully."))
    } else {
        StratCommandResult::err(format!("Strategy `{id}` not found or already stopped."))
    }
}

fn handle_help() -> StratCommandResult {
    StratCommandResult::ok(
        "*Strategy Commands*\n\n\
         `/strat list` — list all strategies with status and P&L\n\
         `/strat status <id>` — detailed metrics for a strategy\n\
         `/strat pnl` — aggregate P&L across all strategies\n\
         `/strat stop <id>` — stop a running strategy (admin only)"
    )
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy_runner::{RunnerConfig, RunnerResult, RunnerStatus, StoredRunner, StrategyRunnerStore};
    use std::sync::Arc;

    fn make_store() -> Arc<StrategyRunnerStore> {
        Arc::new(StrategyRunnerStore::new(std::env::temp_dir()))
    }

    fn make_runner(id: &str, name: &str, kind: &str, pnl_pct: f64) -> StoredRunner {
        let cfg = RunnerConfig {
            id: id.to_string(),
            name: name.to_string(),
            script: String::new(),
            market_type: "polymarket_binary".to_string(),
            symbol: "btc-100k".to_string(),
            interval: "1m".to_string(),
            mode: "paper".to_string(),
            initial_balance: 1000.0,
            fee_pct: 0.1,
            warmup_days: 0,
            auto_restart: false,
            series_id: None, resolution_logic: None, threshold: None,
            poly_creds: None, poly_token_id: None, poly_no_token_id: None,
            poly_condition_id: None, wallet_address: None,
            chainlink_endpoint_url: None, chainlink_api_key: None,
            chainlink_interval_secs: 5,
            live_sizing_mode: Default::default(), live_sizing_value: 0.0,
            stop_loss_pct: None, early_fire_secs: None, max_entry_price: None,
            price_mode: None, max_spread_pct: None,
            kind: Some(kind.to_string()),
            ..Default::default()
        };

        let result = RunnerResult {
            total_return_pct: pnl_pct,
            balance: 1000.0 * (1.0 + pnl_pct / 100.0),
            position: 0.0,
            total_trades: 5,
            win_rate_pct: 60.0,
            sharpe_ratio: 1.2,
            max_drawdown_pct: 5.0,
            all_trades: vec![],
            last_signal: "BUY".to_string(),
            analysis: format!("{kind} analysis"),
            live_feed: None,
            wallet_address: None,
            wallet_balance_usdc: None,
            live_orders: vec![],
            live_wins: 3,
            live_total_trades: 5,
            live_kv_state: std::collections::HashMap::new(),
        };

        StoredRunner {
            config: cfg,
            status: RunnerStatus {
                id: id.to_string(),
                status: "running".to_string(),
                started_at: "2026-05-10T00:00:00Z".to_string(),
                last_tick_at: None,
                next_tick_at: None,
                error: None,
            },
            result: Some(result),
            hidden: false,
        }
    }

    #[test]
    fn strat_list_shows_all_runners() {
        let store = make_store();
        store.upsert(make_runner("r1", "Arb Bot",  "arb_binary",  5.2));
        store.upsert(make_runner("r2", "FV Bot",   "fair_value",  -1.3));

        let res = handle_strat_command("/strat list", false, &store);
        assert!(!res.is_error);
        assert!(res.message.contains("Arb Bot"),  "should list r1");
        assert!(res.message.contains("FV Bot"),   "should list r2");
        assert!(res.message.contains("2 strategy(ies)"));
    }

    #[test]
    fn strat_list_empty_store() {
        let store = make_store();
        let res = handle_strat_command("/strat list", false, &store);
        assert!(!res.is_error);
        assert!(res.message.contains("No strategies"));
    }

    #[test]
    fn strat_status_shows_runner_detail() {
        let store = make_store();
        store.upsert(make_runner("r1", "Arb Bot", "arb_binary", 5.2));

        let res = handle_strat_command("/strat status r1", false, &store);
        assert!(!res.is_error, "should succeed: {}", res.message);
        assert!(res.message.contains("Arb Bot"));
        assert!(res.message.contains("arb_binary"));
        assert!(res.message.contains("60.0%"), "should show win rate");
    }

    #[test]
    fn strat_status_unknown_id_returns_error() {
        let store = make_store();
        let res = handle_strat_command("/strat status unknown-id", false, &store);
        assert!(res.is_error);
        assert!(res.message.contains("not found"));
    }

    #[test]
    fn strat_pnl_aggregates_returns() {
        let store = make_store();
        store.upsert(make_runner("r1", "Arb Bot", "arb_binary",  10.0));
        store.upsert(make_runner("r2", "FV Bot",  "fair_value",  -2.0));

        let res = handle_strat_command("/strat pnl", false, &store);
        assert!(!res.is_error);
        assert!(res.message.contains("P&L Summary"));
        assert!(res.message.contains("2 strategies"));
        assert!(res.message.contains("Total invested"));
    }

    #[test]
    fn strat_pnl_empty_returns_zero() {
        let store = make_store();
        let res = handle_strat_command("/strat pnl", false, &store);
        assert!(!res.is_error);
        assert!(res.message.contains("No strategies to report"));
    }

    #[test]
    fn strat_stop_requires_admin() {
        let store = make_store();
        store.upsert(make_runner("r1", "Bot", "arb_binary", 0.0));

        let res = handle_strat_command("/strat stop r1", false, &store);
        assert!(res.is_error);
        assert!(res.message.contains("Permission denied"));
    }

    #[test]
    fn strat_stop_unknown_runner_returns_error() {
        let store = make_store();
        let res = handle_strat_command("/strat stop unknown-id", true, &store);
        assert!(res.is_error);
        assert!(res.message.contains("not found"));
    }

    #[test]
    fn strat_help_shows_all_commands() {
        let store = make_store();
        let res = handle_strat_command("/strat", false, &store);
        assert!(!res.is_error);
        assert!(res.message.contains("list"));
        assert!(res.message.contains("status"));
        assert!(res.message.contains("pnl"));
        assert!(res.message.contains("stop"));
    }

    #[test]
    fn strat_unknown_subcommand_shows_help() {
        let store = make_store();
        let res = handle_strat_command("/strat foobar", false, &store);
        assert!(!res.is_error);
        assert!(res.message.contains("list"));
    }
}

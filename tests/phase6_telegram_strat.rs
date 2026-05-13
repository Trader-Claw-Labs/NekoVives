//! Phase 6 tests — Telegram /strat command handler.
//!
//! Covers: list, status, pnl, stop, permission checks, unknown commands, and help.

use std::sync::Arc;
use trader_claw::strategy_runner::{
    RunnerConfig, RunnerResult, RunnerStatus, StoredRunner, StrategyRunnerStore,
};
use trader_claw::channels::strat_commands::handle_strat_command;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_store() -> Arc<StrategyRunnerStore> {
    // Each call gets its own directory so parallel tests don't share persisted state.
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("tc-strat-test-{nanos:010}-{:?}", std::thread::current().id()));
    std::fs::create_dir_all(&dir).ok();
    Arc::new(StrategyRunnerStore::new(dir))
}

fn make_runner(id: &str, name: &str, kind: &str, balance: f64, pnl_pct: f64) -> StoredRunner {
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
    };

    let result = RunnerResult {
        total_return_pct: pnl_pct,
        balance,
        position: 0.0,
        total_trades: 7,
        win_rate_pct: 57.1,
        sharpe_ratio: 1.4,
        max_drawdown_pct: 4.0,
        all_trades: vec![],
        last_signal: "HOLD".to_string(),
        analysis: format!("{kind}: 7 trades"),
        live_feed: None,
        wallet_address: None,
        wallet_balance_usdc: None,
        live_orders: vec![],
        live_wins: 4,
        live_total_trades: 7,
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
    }
}

// ── /strat list ───────────────────────────────────────────────────────────────

#[test]
fn strat_list_shows_all_runners() {
    let store = make_store();
    store.upsert(make_runner("r1", "Arb Bot",      "arb_binary",         1050.0, 5.0));
    store.upsert(make_runner("r2", "FV Bot",       "fair_value",          980.0, -2.0));
    store.upsert(make_runner("r3", "Rotation Bot", "rotation_compounder", 1100.0, 10.0));

    let res = handle_strat_command("/strat list", false, &store);
    assert!(!res.is_error, "list should succeed");
    assert!(res.message.contains("Arb Bot"),      "should list r1");
    assert!(res.message.contains("FV Bot"),       "should list r2");
    assert!(res.message.contains("Rotation Bot"), "should list r3");
    assert!(res.message.contains("3 strategy(ies)"));
}

#[test]
fn strat_list_empty_store_returns_friendly_message() {
    let store = make_store();
    let res = handle_strat_command("/strat list", false, &store);
    assert!(!res.is_error);
    assert!(res.message.contains("No strategies"));
}

#[test]
fn strat_list_shows_engine_kind() {
    let store = make_store();
    store.upsert(make_runner("r1", "Hedge Bot", "arb_hedge", 1000.0, 0.0));

    let res = handle_strat_command("/strat list", false, &store);
    assert!(res.message.contains("arb_hedge"), "should show engine kind");
}

// ── /strat status ─────────────────────────────────────────────────────────────

#[test]
fn strat_status_shows_full_metrics() {
    let store = make_store();
    store.upsert(make_runner("fv-1", "FV Runner", "fair_value", 1050.0, 5.0));

    let res = handle_strat_command("/strat status fv-1", false, &store);
    assert!(!res.is_error, "status should succeed: {}", res.message);
    assert!(res.message.contains("FV Runner"));
    assert!(res.message.contains("fair_value"));
    assert!(res.message.contains("57.1%"), "should contain win rate");
    assert!(res.message.contains("btc-100k"), "should contain market symbol");
}

#[test]
fn strat_status_unknown_id_returns_error() {
    let store = make_store();
    let res = handle_strat_command("/strat status ghost-runner", false, &store);
    assert!(res.is_error);
    assert!(res.message.contains("not found"));
}

#[test]
fn strat_status_missing_id_shows_usage() {
    let store = make_store();
    let res = handle_strat_command("/strat status", false, &store);
    assert!(res.is_error);
    assert!(res.message.contains("Usage"));
}

// ── /strat pnl ────────────────────────────────────────────────────────────────

#[test]
fn strat_pnl_aggregates_across_runners() {
    let store = make_store();
    // total invested = 2000, balance = 1100 + 900 = 2000, pnl = 0%
    store.upsert(make_runner("r1", "Win Bot",  "arb_binary",  1100.0,  10.0));
    store.upsert(make_runner("r2", "Loss Bot", "fair_value",   900.0, -10.0));

    let res = handle_strat_command("/strat pnl", false, &store);
    assert!(!res.is_error);
    assert!(res.message.contains("P&L Summary"));
    assert!(res.message.contains("2 strategies"));
    assert!(res.message.contains("$2000.00"), "should show total invested");
    assert!(res.message.contains("Win Bot"),  "should identify best runner");
    assert!(res.message.contains("Loss Bot"), "should identify worst runner");
}

#[test]
fn strat_pnl_empty_store_returns_friendly_message() {
    let store = make_store();
    let res = handle_strat_command("/strat pnl", false, &store);
    assert!(!res.is_error);
    assert!(res.message.contains("No strategies to report"));
}

// ── /strat stop ───────────────────────────────────────────────────────────────

#[test]
fn strat_stop_requires_admin() {
    let store = make_store();
    store.upsert(make_runner("r1", "Bot", "arb_binary", 1000.0, 0.0));

    let res = handle_strat_command("/strat stop r1", false, &store);
    assert!(res.is_error, "non-admin should be denied");
    assert!(res.message.contains("Permission denied"));
}

#[test]
fn strat_stop_unknown_runner_returns_error() {
    let store = make_store();
    let res = handle_strat_command("/strat stop ghost-id", true, &store);
    assert!(res.is_error);
    assert!(res.message.contains("not found"));
}

#[test]
fn strat_stop_missing_id_shows_usage() {
    let store = make_store();
    let res = handle_strat_command("/strat stop", true, &store);
    assert!(res.is_error);
    assert!(res.message.contains("Usage"));
}

// ── Help / unknown ────────────────────────────────────────────────────────────

#[test]
fn strat_bare_command_shows_help() {
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
    let res = handle_strat_command("/strat unknowncmd", false, &store);
    assert!(!res.is_error);
    assert!(res.message.contains("list"));
}

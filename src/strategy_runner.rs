//! Live Strategy Runner — real-time paper/live trading sessions.
//!
//! Each runner:
//!  1. Fetches a warmup window of recent candles from Binance REST.
//!  2. Connects to the Binance WebSocket kline stream for real-time closed candles.
//!  3. Runs the Rhai strategy on a rolling buffer after every new closed candle.
//!  4. In paper mode: tracks simulated P&L and updates the store.
//!     In live mode: sends real orders via Polymarket CLOB API or Hyperliquid CEX.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use serde::{Deserialize, Serialize};
use tokio::task::AbortHandle;

// ── Per-series order serialization (P3B) ─────────────────────────────────────
// One semaphore per series_id ensures parallel runners on the same series
// cannot overlap order submission, preventing book impact from concurrent fills.

static ORDER_QUEUES: std::sync::OnceLock<Arc<Mutex<std::collections::HashMap<String, Arc<tokio::sync::Semaphore>>>>> = std::sync::OnceLock::new();

fn get_series_semaphore(series_id: &str) -> Arc<tokio::sync::Semaphore> {
    let queues = ORDER_QUEUES.get_or_init(|| {
        Arc::new(Mutex::new(std::collections::HashMap::new()))
    });
    let mut map = queues.lock().unwrap();
    map.entry(series_id.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(1)))
        .clone()
}

// ── Global CLOB API rate limiter (P3C) ───────────────────────────────────────
// Cap concurrent CLOB API calls at 5 to avoid 429 / rate-limit errors.

static CLOB_RATE_LIMIT: std::sync::OnceLock<Arc<tokio::sync::Semaphore>> = std::sync::OnceLock::new();

fn clob_semaphore() -> Arc<tokio::sync::Semaphore> {
    CLOB_RATE_LIMIT.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(5))).clone()
}

/// The price used for P&L settlement, shared by the provisional settle, the official
/// re-resolution sweep, and the resolution monitor so they never disagree. Prefers the
/// book-VWAP `fill_price` when it is a sane market-buy fill (in [0.01,0.99] and within
/// 25% of the decision ask — captures real slippage); otherwise falls back to the
/// decision `entry_price`, floored to 0.10 (or 0.50 when entry is itself below 0.10).
/// Using the raw entry ask inflates every win's P&L vs the realistic fill.
pub(crate) fn settle_price(entry_price: Option<f64>, fill_price: Option<f64>) -> f64 {
    let decision_ep = entry_price.unwrap_or(0.5);
    if let Some(fp) = fill_price {
        if fp >= 0.01 && fp <= 0.99 && (decision_ep <= 0.0 || fp <= decision_ep * 1.25) {
            return fp;
        }
    }
    if decision_ep < 0.10 { 0.50 } else { decision_ep.max(0.10) }
}

// ── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LiveSizingMode {
    #[default]
    Percent,
    Fixed,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RunnerConfig {
    pub id: String,
    pub name: String,
    pub script: String,
    pub market_type: String,
    pub symbol: String,
    pub interval: String,
    pub mode: String,
    pub initial_balance: f64,
    pub fee_pct: f64,
    pub warmup_days: u32,
    #[serde(default = "default_auto_restart")]
    pub auto_restart: bool,
    pub series_id: Option<String>,
    pub resolution_logic: Option<String>,
    pub threshold: Option<f64>,
    /// Polymarket CLOB credentials — populated from config when mode = "live".
    /// Never serialised to disk to avoid leaking secrets.
    #[serde(skip)]
    pub poly_creds: Option<polymarket_trader::auth::PolyCredentials>,
    /// Polymarket token_id for the active market slug (resolved at start).
    #[serde(skip)]
    pub poly_token_id: Option<String>,
    /// Polymarket NO token_id for the active market slug (resolved at start).
    #[serde(skip)]
    pub poly_no_token_id: Option<String>,
    /// Polymarket condition_id for the active market slug (resolved at start).
    #[serde(skip)]
    pub poly_condition_id: Option<String>,
    /// Wallet address for live mode (read from config on creation).
    #[serde(skip)]
    pub wallet_address: Option<String>,
    /// Chainlink Data Streams endpoint URL for live price feed (overrides Binance display price).
    #[serde(default)]
    pub chainlink_endpoint_url: Option<String>,
    /// Chainlink API key for authenticated endpoints.
    #[serde(skip)]
    pub chainlink_api_key: Option<String>,
    /// Chainlink polling interval in seconds.
    #[serde(default = "default_chainlink_interval")]
    pub chainlink_interval_secs: u64,
    /// Live order sizing mode: "fixed" = USD amount, "percent" = % of balance.
    #[serde(default)]
    pub live_sizing_mode: LiveSizingMode,
    /// Live order sizing value: USD amount if fixed, decimal fraction if percent.
    #[serde(default)]
    pub live_sizing_value: f64,
    /// Stop-loss threshold: exit position early if token price drops this fraction
    /// from entry (e.g. 0.40 = exit if we lose 40% of position value).
    /// None = disabled. Only active in live polymarket_binary mode.
    #[serde(default)]
    pub stop_loss_pct: Option<f64>,
    /// Fire order N seconds before the decision candle closes (0 = at close).
    /// Overrides the global [live_strategy] early_fire_secs from config.toml.
    #[serde(default)]
    pub early_fire_secs: Option<u32>,
    /// Maximum token entry price. If the current token price exceeds this
    /// value, the trade/bet is skipped. Applies to live and paper modes.
    #[serde(default)]
    pub max_entry_price: Option<f64>,
    /// Price mode for Polymarket binary entry: "historical" = buy price from CLOB,
    /// "mid" = average of buy/sell (mid-price).
    #[serde(default)]
    pub price_mode: Option<String>,
    /// Maximum tolerated deviation of `yes_mid + no_mid` from 1.0 before the
    /// runner skips the decision window. 0.03 = 3% = ~6¢ of combined spread.
    /// Rationale: BT assumes `no = 1 - yes`. When live books are wide (low
    /// liquidity / high uncertainty), paper fills at mid are optimistic vs.
    /// what real execution would cost. Default is 0.03; set to a large value
    /// like 1.0 to disable the gate entirely.
    #[serde(default)]
    pub max_spread_pct: Option<f64>,
    /// Maximum slippage % accepted for live market orders.
    /// worst_price = mid * (1 + max_slippage_pct / 100).
    /// If the order can only fill above this ceiling, the CLOB rejects it.
    /// Default 10.0 (10%). Set lower to be stricter; set higher for thin books.
    #[serde(default)]
    pub max_slippage_pct: Option<f64>,
    /// Maximum kelly multiplier. Prevents scripts from scaling bets beyond this
    /// multiple of the base stake. Default 1.5 (50% above base). Was hardcoded 2.0.
    #[serde(default = "default_kelly_cap")]
    pub kelly_size_cap: f64,
    /// Auto-stop when cumulative live P&L loss exceeds this % of initial_balance.
    /// E.g. 0.40 = stop when down 40%. Default disabled (0.0).
    #[serde(default)]
    pub max_runner_loss_pct: Option<f64>,
    /// Auto-stop after N consecutive losses without a win. Default disabled (0).
    #[serde(default)]
    pub max_consecutive_losses: Option<u32>,
    /// Minimum entry price (token ask price). Skip orders when ep < this value.
    /// Prevents extreme long-shot bets at 3-5% probability. Default 0.05.
    #[serde(default = "default_min_entry_price")]
    pub min_entry_price: f64,
    /// Allowed UTC hours (0-23). When set, the runner skips decision windows
    /// whose hour is NOT in this list. Empirically, hours 01/03/04/07/08/14/17/20
    /// (UTC) show ~34% WR vs 56% for the best hours. Empty vec = no restriction.
    #[serde(default)]
    pub allowed_hours: Vec<u8>,
    /// Minimum BTC realized-vol (60-period stdev of 1m log-returns) required
    /// before the runner will place a bet. Flat markets degrade drift signal
    /// to noise. Empirical threshold: 0.00015 (half of normal baseline).
    /// 0.0 / None = disabled.
    #[serde(default)]
    pub rv_min_btc: Option<f64>,
    /// Hyperliquid signer for live CEX trading. Created at runner start from
    /// wallet-manager decrypted key. Never serialized to disk.
    #[serde(skip)]
    pub hl_signer: Option<hyperliquid_trader::exchange::Signer>,
    /// Trading risk gate reference for live mode. Passed from AppState.
    #[serde(skip)]
    pub risk_gate: Option<Arc<risk_manager::general::TradingRiskGate>>,
    /// Binance Futures credentials for live CEX trading.
    /// If present and hl_signer is None, the runner trades on Binance instead of Hyperliquid.
    #[serde(skip)]
    pub binance_creds: Option<crate::tools::binance_perps::BinanceCredentials>,
    // ── Funding arbitrage configuration ───────────────────────────────────────
    /// Watchlist of coins to monitor for funding rate arbitrage.
    #[serde(default = "default_funding_watchlist")]
    pub funding_watchlist: Vec<String>,
    /// Minimum APR difference required to open a funding arb position.
    #[serde(default = "default_min_apr_diff")]
    pub min_apr_diff: f64,
    /// APR diff threshold below which an open position is force-closed.
    #[serde(default = "default_force_close_diff")]
    pub force_close_diff: f64,
    /// Maximum number of concurrent funding arb pairs.
    #[serde(default = "default_max_open_pairs")]
    pub max_open_pairs: usize,
    /// Max % of capital allocated per pair (split across both legs).
    #[serde(default = "default_max_pos_pct")]
    pub max_pos_pct: f64,
    /// Polling interval in seconds for funding rate checks.
    #[serde(default = "default_funding_poll_secs")]
    pub funding_poll_secs: u64,
    /// Estimated taker fee + slippage in basis points per leg per cycle.
    #[serde(default = "default_fee_buffer_bps")]
    pub fee_buffer_bps: f64,
    /// Engine kind identifier.  `None` (or "rhai_candle") = legacy Rhai path.
    /// New values: "arb_binary", "minting_mm", "rotation_compounder",
    /// "fair_value", "fv_momentum".  Serialised configs without this field
    /// continue to work unchanged because of the serde default.
    #[serde(default)]
    pub kind: Option<String>,
    /// Per-engine tunable parameters (UI EngineParamsForm).  Merged over
    /// each engine's defaults at runner start so the live loop honours the
    /// edge / threshold / sizing knobs the user picked. JSON-shaped so we
    /// don't couple this struct to engine-specific config types.
    #[serde(default)]
    pub engine_params: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerStatus {
    pub id: String,
    pub status: String,
    pub started_at: String,
    pub last_tick_at: Option<String>,
    pub next_tick_at: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LiveOrder {
    pub timestamp: String,
    pub window_ts: i64,
    pub side: String,
    pub token_id: String,
    pub amount_usdc: f64,
    pub order_id: String,
    pub status: String,
    /// Decision-time orderbook midpoint. Kept for diagnostics; do NOT use for
    /// P&L. The real fill price the chain actually paid is `fill_price`.
    pub entry_price: Option<f64>,
    pub result: Option<String>,
    pub pnl: Option<f64>,
    /// True when this position was closed early by the stop-loss monitor.
    #[serde(default)]
    pub stop_loss_triggered: bool,

    // ── Real CLOB fill data (populated from GET /data/trades) ──────────────
    /// Volume-weighted average fill price across all trades that matched this
    /// `order_id`. Populated by the live runner immediately after order
    /// placement and by the historical backfill tool. None until reconciled.
    #[serde(default)]
    pub fill_price: Option<f64>,
    /// Number of outcome shares actually received.
    #[serde(default)]
    pub fill_size: Option<f64>,
    /// Polygon transaction hash of the on-chain match (first matched trade if
    /// the order filled across multiple trades).
    #[serde(default)]
    pub tx_hash: Option<String>,

    // ── Polymarket-derived resolution (replaces Binance candle inference) ──
    /// True when the YES/UP outcome won, per Gamma `outcomePrices`. None when
    /// the market hasn't resolved yet or settlement hasn't been reconciled.
    #[serde(default)]
    pub resolution_yes_won: Option<bool>,
    /// Free-text source: "polymarket" once reconciled via Gamma; "binance" if
    /// settled by the legacy candle path; None until settled.
    #[serde(default)]
    pub resolution_source: Option<String>,

    /// True once the historical-data backfill tool has reconciled this order
    /// against the CLOB and Gamma. Allows incremental re-runs.
    #[serde(default)]
    pub backfilled: bool,

    /// Polymarket condition_id of the market this order traded. Captured at order
    /// time so the resolution monitor can settle via the CLOB by-condition_id
    /// lookup — the legacy `{prefix}-{window_ts}` Gamma slug no longer resolves
    /// for recurring 5m/15m markets, which left every non-BTC runner stuck on the
    /// unreliable binance_provisional resolution.
    #[serde(default)]
    pub condition_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerResult {
    pub total_return_pct: f64,
    pub balance: f64,
    pub position: f64,
    pub total_trades: u32,
    pub win_rate_pct: f64,
    pub sharpe_ratio: f64,
    pub max_drawdown_pct: f64,
    pub all_trades: Vec<crate::tools::backtest::AllTrade>,
    pub last_signal: String,
    pub analysis: String,
    pub live_feed: Option<LiveFeedData>,
    pub wallet_address: Option<String>,
    pub wallet_balance_usdc: Option<f64>,
    /// Live-mode orders placed via Polymarket CLOB (reset on runner start)
    pub live_orders: Vec<LiveOrder>,
    /// Live-mode win count (for calculating live win rate)
    pub live_wins: u32,
    /// Live-mode total trades count
    pub live_total_trades: u32,
    /// Persistent script kv_state across pause/restart cycles. Mirrors
    /// `ctx.set(...)` values that the Rhai script carries between windows
    /// (avg_vol, loss_streak, pause_until, etc.). Empty `{}` on fresh runner.
    #[serde(default)]
    pub live_kv_state: std::collections::HashMap<String, f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveFeedData {
    pub current_btc_price: f64,
    pub market_slug: String,
    pub window_timestamp: i64,
    pub window_seconds_left: i64,
    pub price_to_beat: f64,
    pub yes_token_price: f64,
    pub no_token_price: f64,
    /// Last 60 seconds of BTC price points for the mini chart
    pub price_history: Vec<(i64, f64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredRunner {
    pub config: RunnerConfig,
    pub status: RunnerStatus,
    pub result: Option<RunnerResult>,
    #[serde(default)]
    pub hidden: bool,
}

pub struct StrategyRunnerStore {
    runners: Arc<Mutex<std::collections::HashMap<String, StoredRunner>>>,
    handles: Arc<Mutex<std::collections::HashMap<String, AbortHandle>>>,
    workspace_dir: PathBuf,
}

fn default_auto_restart() -> bool { true }
fn default_chainlink_interval() -> u64 { 5 }
fn default_kelly_cap() -> f64 { 1.5 }
fn default_min_entry_price() -> f64 { 0.05 }

fn default_funding_watchlist() -> Vec<String> {
    vec!["BTC".into(), "ETH".into(), "SOL".into(), "AVAX".into()]
}
fn default_min_apr_diff() -> f64 { 0.10 }
fn default_force_close_diff() -> f64 { 0.02 }
fn default_max_open_pairs() -> usize { 4 }
fn default_max_pos_pct() -> f64 { 0.15 }
fn default_funding_poll_secs() -> u64 { 60 }
fn default_fee_buffer_bps() -> f64 { 12.0 }

// ── Store impl ───────────────────────────────────────────────────────────────

impl StrategyRunnerStore {
    pub fn new(workspace_dir: PathBuf) -> Self {
        let store = Self {
            runners: Arc::new(Mutex::new(std::collections::HashMap::new())),
            handles: Arc::new(Mutex::new(std::collections::HashMap::new())),
            workspace_dir,
        };
        store.load_from_disk();
        store
    }

    fn runners_file(&self) -> PathBuf {
        self.workspace_dir.join("live_strategies.json")
    }

    fn load_from_disk(&self) {
        if let Ok(data) = std::fs::read_to_string(self.runners_file()) {
            if let Ok(runners) = serde_json::from_str::<Vec<StoredRunner>>(&data) {
                let mut map = self.runners.lock().unwrap();
                for mut r in runners {
                    let was_running = r.status.status == "running" || r.status.status == "starting";
                    if was_running {
                        r.status.status = if r.config.auto_restart { "starting" } else { "stopped" }.to_string();
                    }
                    backfill_engine_series_id(&mut r.config);
                    map.insert(r.config.id.clone(), r);
                }
            }
        }
    }

    pub fn list_restartable_configs(&self) -> Vec<RunnerConfig> {
        self.runners
            .lock()
            .unwrap()
            .values()
            .filter(|r| r.status.status == "starting" && r.config.auto_restart)
            .map(|r| r.config.clone())
            .collect()
    }

    pub fn set_auto_restart(&self, id: &str, auto_restart: bool) -> Option<StoredRunner> {
        let mut map = self.runners.lock().unwrap();
        let updated = map.get_mut(id).map(|r| {
            r.config.auto_restart = auto_restart;
            if !auto_restart && (r.status.status == "starting") {
                r.status.status = "stopped".to_string();
            }
            r.clone()
        });
        drop(map);
        if updated.is_some() {
            self.persist();
        }
        updated
    }

    pub fn set_hidden(&self, id: &str, hidden: bool) -> Option<StoredRunner> {
        let mut map = self.runners.lock().unwrap();
        let updated = map.get_mut(id).map(|r| {
            r.hidden = hidden;
            r.clone()
        });
        drop(map);
        if updated.is_some() {
            self.persist();
        }
        updated
    }

    pub fn update_runner_config(
        &self,
        id: &str,
        live_sizing_mode: Option<LiveSizingMode>,
        live_sizing_value: Option<f64>,
        // Option<Option<T>>: None = absent (skip), Some(None) = clear, Some(Some(v)) = set
        max_entry_price: Option<Option<f64>>,
        allowed_hours: Option<Vec<u8>>,
        rv_min_btc: Option<Option<f64>>,
        price_mode: Option<String>,
        max_spread_pct: Option<Option<f64>>,
        max_slippage_pct: Option<Option<f64>>,
        early_fire_secs: Option<Option<u32>>,
        kelly_size_cap: Option<f64>,
        max_runner_loss_pct: Option<f64>,
        max_consecutive_losses: Option<u32>,
        min_entry_price: Option<f64>,
        stop_loss_pct: Option<Option<f64>>,
    ) -> Option<StoredRunner> {
        let mut map = self.runners.lock().unwrap();
        let updated = map.get_mut(id).map(|r| {
            if let Some(mode) = live_sizing_mode {
                r.config.live_sizing_mode = mode;
            }
            if let Some(val) = live_sizing_value {
                r.config.live_sizing_value = val;
            }
            if let Some(maybe) = max_entry_price {
                r.config.max_entry_price = maybe;
            }
            if let Some(mode) = price_mode {
                r.config.price_mode = Some(mode);
            }
            if let Some(maybe) = max_spread_pct {
                r.config.max_spread_pct = maybe;
            }
            if let Some(maybe) = max_slippage_pct {
                r.config.max_slippage_pct = maybe;
            }
            if let Some(maybe) = early_fire_secs {
                r.config.early_fire_secs = maybe;
            }
            if let Some(hours) = allowed_hours {
                r.config.allowed_hours = hours;
            }
            if let Some(maybe) = rv_min_btc {
                r.config.rv_min_btc = maybe;
            }
            if let Some(cap) = kelly_size_cap {
                r.config.kelly_size_cap = cap;
            }
            if let Some(loss_pct) = max_runner_loss_pct {
                r.config.max_runner_loss_pct = if loss_pct > 0.0 { Some(loss_pct) } else { None };
            }
            if let Some(streak) = max_consecutive_losses {
                r.config.max_consecutive_losses = if streak > 0 { Some(streak) } else { None };
            }
            if let Some(min_ep) = min_entry_price {
                r.config.min_entry_price = min_ep;
            }
            if let Some(maybe_sl) = stop_loss_pct {
                r.config.stop_loss_pct = maybe_sl;
            }
            r.clone()
        });
        drop(map);
        if updated.is_some() {
            self.persist();
        }
        updated
    }

    pub fn restart_previously_running(self: &Arc<Self>, workspace_dir: PathBuf, config_path: Option<PathBuf>) -> usize {
        let configs = self.list_restartable_configs();
        let count = configs.len();
        // Clear stale timestamps so the UI doesn't show a "next tick" in the past
        {
            let mut map = self.runners.lock().unwrap();
            for c in &configs {
                if let Some(r) = map.get_mut(&c.id) {
                    r.status.next_tick_at = None;
                    r.status.last_tick_at = None;
                }
            }
        }
        for config in configs {
            let id = config.id.clone();
            if self.handles.lock().unwrap().contains_key(&id) {
                continue;
            }
            let store = self.clone();
            let ws_dir = workspace_dir.clone();
            let cfg_path = config_path.clone();
            let task = tokio::spawn(async move {
                runner_loop(store, config, ws_dir, cfg_path).await;
            });
            self.register_handle(id, task.abort_handle());
        }
        if count > 0 {
            self.persist();
        }
        count
    }

    pub fn hydrate_live_creds_for_runner(&self, id: &str, creds: polymarket_trader::auth::PolyCredentials) -> bool {
        let mut map = self.runners.lock().unwrap();
        if let Some(r) = map.get_mut(id) {
            r.config.poly_creds = Some(creds);
            true
        } else {
            false
        }
    }

    pub fn set_poly_token_ids(&self, id: &str, yes_token_id: String, no_token_id: String) -> bool {
        let mut map = self.runners.lock().unwrap();
        if let Some(r) = map.get_mut(id) {
            r.config.poly_token_id = Some(yes_token_id);
            r.config.poly_no_token_id = Some(no_token_id);
            true
        } else {
            false
        }
    }

    pub fn set_wallet_address(&self, id: &str, addr: String) -> bool {
        let mut map = self.runners.lock().unwrap();
        if let Some(r) = map.get_mut(id) {
            r.config.wallet_address = Some(addr);
            true
        } else {
            false
        }
    }

    pub fn set_starting(&self, id: &str) -> bool {
        let mut map = self.runners.lock().unwrap();
        if let Some(r) = map.get_mut(id) {
            r.status.status = "starting".to_string();
            r.status.error = None;
            true
        } else {
            false
        }
    }

    pub fn persist_public_config(&self) {
        self.persist();
    }

    pub fn persist(&self) {
        let runners: Vec<StoredRunner> = self.runners.lock().unwrap().values().cloned().collect();
        if let Ok(json) = serde_json::to_string_pretty(&runners) {
            let _ = std::fs::write(self.runners_file(), json);
        }
    }

    pub fn list(&self) -> Vec<StoredRunner> {
        let mut runners: Vec<StoredRunner> = self.runners.lock().unwrap().values().cloned().collect();
        runners.sort_by(|a, b| a.status.started_at.cmp(&b.status.started_at));
        runners
    }

    pub fn get(&self, id: &str) -> Option<StoredRunner> {
        self.runners.lock().unwrap().get(id).cloned()
    }

    pub fn upsert(&self, runner: StoredRunner) {
        self.runners.lock().unwrap().insert(runner.config.id.clone(), runner);
        self.persist();
    }

    pub fn stop(&self, id: &str) -> bool {
        if let Some(handle) = self.handles.lock().unwrap().remove(id) {
            handle.abort();
        }
        let mut map = self.runners.lock().unwrap();
        if let Some(r) = map.get_mut(id) {
            r.status.status = "stopped".to_string();
            drop(map);
            self.persist();
            true
        } else {
            false
        }
    }

    /// Stop ALL currently-running live-mode runners. Called by the portfolio guard
    /// when the global max-loss threshold is breached.
    pub fn stop_all_live(&self) -> Vec<String> {
        let ids: Vec<String> = {
            let map = self.runners.lock().unwrap();
            map.values()
                .filter(|r| r.config.mode == "live" && r.status.status == "running")
                .map(|r| r.config.id.clone())
                .collect()
        };
        let stopped: Vec<String> = ids.iter().filter(|id| self.stop(id)).cloned().collect();
        if !stopped.is_empty() {
            tracing::warn!(
                "[PORTFOLIO_GUARD] Emergency stop triggered. Stopped {} live runners: {:?}",
                stopped.len(), stopped
            );
        }
        stopped
    }

    pub fn delete(&self, id: &str) -> bool {
        self.stop(id);
        let removed = self.runners.lock().unwrap().remove(id).is_some();
        self.persist();
        removed
    }

    pub fn register_handle(&self, id: String, handle: AbortHandle) {
        self.handles.lock().unwrap().insert(id, handle);
    }

    /// Mutate the `RunnerResult` of an existing runner in-place.
    /// Creates a default result if none exists yet.  Used by new-style engines.
    pub fn update_result<F>(&self, id: &str, f: F)
    where
        F: FnOnce(&mut RunnerResult),
    {
        let mut map = self.runners.lock().unwrap();
        if let Some(runner) = map.get_mut(id) {
            let result = runner.result.get_or_insert_with(|| RunnerResult {
                total_return_pct: 0.0,
                balance: runner.config.initial_balance,
                position: 0.0,
                total_trades: 0,
                win_rate_pct: 0.0,
                sharpe_ratio: 0.0,
                max_drawdown_pct: 0.0,
                last_signal: "none".to_string(),
                analysis: String::new(),
                live_feed: None,
                wallet_address: None,
                wallet_balance_usdc: None,
                live_orders: vec![],
                live_wins: 0,
                live_total_trades: 0,
                all_trades: vec![],
                live_kv_state: std::collections::HashMap::new(),
            });
            f(result);
            runner.status.last_tick_at = Some(chrono::Utc::now().to_rfc3339());
        }
        drop(map);
        self.persist();
    }
}

/// Repair runners persisted before the engine kinds learned to consume
/// `series_id`. Older configs were saved with `kind=arb_binary, symbol="BTCUSDT",
/// series_id=""`; the engine then tries to resolve "BTCUSDT" as a Polymarket
/// slug every poll and logs `No active market with valid tokens for slug:
/// BTCUSDT`. If the symbol matches a built-in series we backfill `series_id`
/// in-place so the engine resolves the current window slug instead.
fn backfill_engine_series_id(config: &mut RunnerConfig) {
    let kind = config.kind.as_deref().unwrap_or("rhai_candle");
    if kind == "rhai_candle" || kind.is_empty() {
        return;
    }
    let needs_backfill = config
        .series_id
        .as_deref()
        .map(str::is_empty)
        .unwrap_or(true);
    if !needs_backfill {
        return;
    }
    // Match symbol → cadence to a builtin series (default to 5m when symbol
    // alone is ambiguous, since that's the cadence every engine kind targets).
    let symbol_upper = config.symbol.split(',').next().unwrap_or("").trim().to_uppercase();
    if symbol_upper.is_empty() {
        return;
    }
    let cadence = if config.interval.is_empty() { "5m" } else { config.interval.as_str() };
    if let Some(series) = crate::tools::series::builtin_series()
        .into_iter()
        .find(|s| s.symbol.eq_ignore_ascii_case(&symbol_upper) && s.cadence == cadence)
        .or_else(|| crate::tools::series::builtin_series()
            .into_iter()
            .find(|s| s.symbol.eq_ignore_ascii_case(&symbol_upper)))
    {
        tracing::info!(
            "[RUNNER {}] Backfilled series_id='{}' for engine kind '{}' (symbol={}, was empty)",
            config.id, series.id, kind, symbol_upper
        );
        config.series_id = Some(series.id);
    }
}

// ── Start a new runner ───────────────────────────────────────────────────────

pub fn start_runner(
    store: Arc<StrategyRunnerStore>,
    mut config: RunnerConfig,
    workspace_dir: PathBuf,
    config_path: Option<PathBuf>,
) -> StoredRunner {
    backfill_engine_series_id(&mut config);
    let id = config.id.clone();
    let now = chrono::Utc::now().to_rfc3339();

    // Preserve any existing result (live_orders, live_wins, live_kv_state, ...)
    // so manual restart / auto-restart on error / restart_previously_running
    // do NOT wipe accumulated stats. The runner loop will rehydrate from this.
    let existing_result = store.get(&id).and_then(|r| r.result);

    let status = RunnerStatus {
        id: id.clone(),
        status: "starting".to_string(),
        started_at: now.clone(),
        last_tick_at: None,
        next_tick_at: None,
        error: None,
    };

    let runner = StoredRunner {
        config: config.clone(),
        status: status.clone(),
        result: existing_result,
        hidden: false,
    };
    store.upsert(runner.clone());

    let store_clone = store.clone();
    let ws_dir = workspace_dir.clone();
    let task = tokio::spawn(async move {
        // Auto-restart wrapper. If `runner_loop` exits with status="error" and
        // the runner has auto_restart=true, wait and restart up to 5 times.
        // Trades and KV state survive because start_runner preserves the
        // result on each upsert.
        const MAX_AUTO_RESTARTS: u32 = 5;
        const RESTART_DELAY_SECS: u64 = 30;
        let mut attempt: u32 = 0;
        loop {
            let cfg = config.clone();
            let cfg_path = config_path.clone();
            let ws = ws_dir.clone();
            let store_inner = store_clone.clone();
            runner_loop(store_inner, cfg, ws, cfg_path).await;

            let should_restart = {
                let map = store_clone.runners.lock().unwrap();
                map.get(&config.id).map(|r| {
                    r.status.status == "error" && r.config.auto_restart
                }).unwrap_or(false)
            };
            if !should_restart {
                break;
            }
            attempt += 1;
            if attempt > MAX_AUTO_RESTARTS {
                tracing::error!(
                    "[RUNNER {}] Auto-restart exhausted ({}/{}). Manual restart required.",
                    config.id, MAX_AUTO_RESTARTS, MAX_AUTO_RESTARTS
                );
                break;
            }
            tracing::info!(
                "[RUNNER {}] Auto-restart attempt {}/{} in {}s",
                config.id, attempt, MAX_AUTO_RESTARTS, RESTART_DELAY_SECS
            );
            // Mark as 'starting' with note so dashboard shows progress.
            {
                let mut map = store_clone.runners.lock().unwrap();
                if let Some(r) = map.get_mut(&config.id) {
                    r.status.status = "starting".to_string();
                    r.status.error = Some(format!(
                        "Auto-restart {}/{} after error", attempt, MAX_AUTO_RESTARTS
                    ));
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(RESTART_DELAY_SECS)).await;
            // Clear error message before re-entering loop.
            {
                let mut map = store_clone.runners.lock().unwrap();
                if let Some(r) = map.get_mut(&config.id) {
                    r.status.error = None;
                }
            }
        }
    });
    store.register_handle(id, task.abort_handle());

    runner
}

// ── Background runner loop ────────────────────────────────────────────────────

async fn runner_loop(
    store: Arc<StrategyRunnerStore>,
    config: RunnerConfig,
    workspace_dir: PathBuf,
    config_path: Option<PathBuf>,
) {
    // Dispatch to the correct loop based on engine kind (new) or market_type (legacy).
    // New engine kinds short-circuit before the legacy market_type dispatch so that
    // existing runners continue to work without any config changes.
    let kind = config.kind.as_deref().unwrap_or(strategy_core::engines::RHAI_CANDLE);
    match kind {
        strategy_core::engines::ARB_BINARY => {
            crate::engines::arb_binary::run_arb_binary_loop(store, config, workspace_dir).await;
        }
        strategy_core::engines::MINTING_MM => {
            crate::engines::minting_mm::run_minting_mm_loop(store, config, workspace_dir).await;
        }
        strategy_core::engines::ROTATION_COMPOUNDER => {
            crate::engines::rotation_compounder::run_rotation_compounder_loop(store, config, workspace_dir).await;
        }
        strategy_core::engines::FAIR_VALUE => {
            crate::engines::fair_value::run_fair_value_loop(store, config, workspace_dir).await;
        }
        strategy_core::engines::FV_MOMENTUM => {
            crate::engines::fv_momentum::run_fv_momentum_loop(store, config, workspace_dir).await;
        }
        strategy_core::engines::ARB_HEDGE => {
            crate::engines::arb_hedge::run_arb_hedge_loop(store, config, workspace_dir).await;
        }
        strategy_core::engines::RHAI_TICK => {
            tick_runner_loop(store, config, workspace_dir).await;
        }
        // "rhai_candle" or None → legacy path (unchanged behaviour)
        _ => {
            if config.market_type == "polymarket_binary" {
                polymarket_runner_loop(store, config, workspace_dir, config_path).await;
            } else if config.market_type == "funding_arb" {
                funding_arb_runner_loop(store, config, workspace_dir).await;
            } else {
                crypto_runner_loop(store, config, workspace_dir).await;
            }
        }
    }
}

/// Compute simple ATR over the last `period` candles.
fn compute_atr_simple(candles: &[crate::tools::backtest::Candle], period: usize) -> f64 {
    if candles.len() < period + 1 {
        return 0.0;
    }
    let mut tr_sum = 0.0;
    for i in (candles.len() - period)..candles.len() {
        let prev_close = candles[i - 1].close;
        let high = candles[i].high;
        let low = candles[i].low;
        let tr = (high - low)
            .max((high - prev_close).abs())
            .max((low - prev_close).abs());
        tr_sum += tr;
    }
    tr_sum / period as f64
}

/// Convert a Binance-style symbol (e.g. "BTCUSDT") to a Hyperliquid coin ("BTC").
fn binance_symbol_to_hl_coin(symbol: &str) -> String {
    symbol
        .trim_end_matches("USDT")
        .trim_end_matches("USDC")
        .trim_end_matches("USD")
        .to_string()
}

/// Execute a CEX position change via Hyperliquid.
/// `prev_pos` and `new_pos` are the strategy-engine position states.
/// Positive = long, negative = short, zero = flat.
async fn execute_hl_position_change(
    client: &hyperliquid_trader::HyperliquidClient,
    gate: &risk_manager::general::TradingRiskGate,
    config: &RunnerConfig,
    coin: &str,
    price: f64,
    prev_pos: f64,
    new_pos: f64,
    atr14: f64,
    live_orders: &mut Vec<LiveOrder>,
) {
    if (new_pos - prev_pos).abs() < f64::EPSILON {
        return;
    }

    let side_str;
    let order_result: Result<hyperliquid_trader::types::OrderResponse, String>;
    let size_usd: f64;

    // Determine sizing
    let capital = gate.status().total_capital.max(config.initial_balance);
    size_usd = match config.live_sizing_mode {
        LiveSizingMode::Fixed => config.live_sizing_value,
        LiveSizingMode::Percent => capital * config.live_sizing_value,
    };

    if size_usd <= 0.0 {
        tracing::warn!("[HL LIVE] Sizing produced zero or negative size; skipping");
        return;
    }

    // Risk gate approval
    let order_req = risk_manager::general::OrderRequest {
        symbol: coin.to_string(),
        strategy_id: config.id.clone(),
        side: if new_pos > 0.0 { "buy".into() } else { "sell".into() },
        proposed_size_usd: size_usd,
        stop_distance_atr: 2.0,
        atr14,
        is_memecoin: false,
    };
    let ctx = risk_manager::general::OrderContext { current_price: price };

    let approved = match gate.approve_order(&order_req, &ctx) {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!("[HL LIVE] Risk gate rejected order: {e}");
            live_orders.push(LiveOrder {
                timestamp: chrono::Utc::now().to_rfc3339(),
                window_ts: 0,
                side: order_req.side.clone(),
                token_id: coin.to_string(),
                amount_usdc: size_usd,
                order_id: "REJECTED".to_string(),
                status: format!("rejected: {e}"),
                entry_price: Some(price),
                result: None,
                pnl: None,
                stop_loss_triggered: false,
                ..Default::default()
            });
            return;
        }
    };

    let exec_size_usd = approved.approved_size_usd;
    let sz_coins = exec_size_usd / price;

    // Handle transitions
    let prev_side = if prev_pos > 0.0 { "long" } else if prev_pos < 0.0 { "short" } else { "flat" };
    let new_side = if new_pos > 0.0 { "long" } else if new_pos < 0.0 { "short" } else { "flat" };

    if new_side == "flat" {
        // Close existing position
        side_str = "close".to_string();
        tracing::info!("[HL LIVE] Closing position for {coin} (was {prev_side})");
        order_result = client
            .close_position(coin)
            .await
            .map_err(|e| e.to_string());
    } else if prev_side == "flat" {
        // Open new position
        side_str = if new_side == "long" { "buy" } else { "sell" }.to_string();
        let order = if new_side == "long" {
            hyperliquid_trader::types::Order::market_buy(coin, sz_coins)
        } else {
            hyperliquid_trader::types::Order::market_sell(coin, sz_coins)
        };
        tracing::info!("[HL LIVE] Opening {new_side} {coin} @ ~{price:.2} | {exec_size_usd:.2} USD ({sz_coins:.6} coins)");
        order_result = client.place_order(&order).await.map_err(|e| e.to_string());
    } else if prev_side != new_side {
        // Reverse: close then open
        side_str = format!("reverse_{}_to_{}", prev_side, new_side);
        tracing::info!("[HL LIVE] Reversing {prev_side} → {new_side} for {coin}");
        let _ = client.close_position(coin).await;
        let order = if new_side == "long" {
            hyperliquid_trader::types::Order::market_buy(coin, sz_coins)
        } else {
            hyperliquid_trader::types::Order::market_sell(coin, sz_coins)
        };
        order_result = client.place_order(&order).await.map_err(|e| e.to_string());
    } else {
        // Same direction but size might have changed — for simplicity, skip
        return;
    }

    let timestamp = chrono::Utc::now().to_rfc3339();
    match order_result {
        Ok(resp) => {
            let oid = resp.order_id.map(|id| id.to_string()).unwrap_or_else(|| "ok".to_string());
            tracing::info!("[HL LIVE] Order placed: {oid}");
            gate.record_fill(&risk_manager::general::FillRecord {
                symbol: coin.to_string(),
                strategy_id: config.id.clone(),
                side: side_str.clone(),
                size_usd: exec_size_usd,
                price,
                pnl_realized: 0.0,
                is_memecoin: false,
                timestamp: chrono::Utc::now(),
            });
            live_orders.push(LiveOrder {
                timestamp,
                window_ts: 0,
                side: side_str,
                token_id: coin.to_string(),
                amount_usdc: exec_size_usd,
                order_id: oid,
                status: "filled".to_string(),
                entry_price: Some(price),
                result: None,
                pnl: None,
                stop_loss_triggered: false,
                ..Default::default()
            });
        }
        Err(e) => {
            tracing::error!("[HL LIVE] Order failed: {e}");
            live_orders.push(LiveOrder {
                timestamp,
                window_ts: 0,
                side: side_str,
                token_id: coin.to_string(),
                amount_usdc: exec_size_usd,
                order_id: "FAILED".to_string(),
                status: format!("error: {e}"),
                entry_price: Some(price),
                result: None,
                pnl: None,
                stop_loss_triggered: false,
                ..Default::default()
            });
        }
    }
}

/// Execute a CEX position change via Binance Futures.
async fn execute_binance_position_change(
    creds: &crate::tools::binance_perps::BinanceCredentials,
    gate: &risk_manager::general::TradingRiskGate,
    config: &RunnerConfig,
    symbol: &str,
    price: f64,
    prev_pos: f64,
    new_pos: f64,
    atr14: f64,
    live_orders: &mut Vec<LiveOrder>,
) {
    if (new_pos - prev_pos).abs() < f64::EPSILON {
        return;
    }

    let side_str;
    let order_result: Result<serde_json::Value, String>;
    let size_usd: f64;

    let capital = gate.status().total_capital.max(config.initial_balance);
    size_usd = match config.live_sizing_mode {
        LiveSizingMode::Fixed => config.live_sizing_value,
        LiveSizingMode::Percent => capital * config.live_sizing_value,
    };

    if size_usd <= 0.0 {
        tracing::warn!("[BN LIVE] Sizing produced zero or negative size; skipping");
        return;
    }

    let order_req = risk_manager::general::OrderRequest {
        symbol: symbol.to_string(),
        strategy_id: config.id.clone(),
        side: if new_pos > 0.0 { "buy".into() } else { "sell".into() },
        proposed_size_usd: size_usd,
        stop_distance_atr: 2.0,
        atr14,
        is_memecoin: false,
    };
    let ctx = risk_manager::general::OrderContext { current_price: price };

    let approved = match gate.approve_order(&order_req, &ctx) {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!("[BN LIVE] Risk gate rejected order: {e}");
            live_orders.push(LiveOrder {
                timestamp: chrono::Utc::now().to_rfc3339(),
                window_ts: 0,
                side: order_req.side.clone(),
                token_id: symbol.to_string(),
                amount_usdc: size_usd,
                order_id: "REJECTED".to_string(),
                status: format!("rejected: {e}"),
                entry_price: Some(price),
                result: None,
                pnl: None,
                stop_loss_triggered: false,
                ..Default::default()
            });
            return;
        }
    };

    let exec_size_usd = approved.approved_size_usd;
    let qty = exec_size_usd / price;

    let prev_side = if prev_pos > 0.0 { "long" } else if prev_pos < 0.0 { "short" } else { "flat" };
    let new_side = if new_pos > 0.0 { "long" } else if new_pos < 0.0 { "short" } else { "flat" };

    if new_side == "flat" {
        side_str = "close".to_string();
        tracing::info!("[BN LIVE] Closing position for {symbol} (was {prev_side})");
        order_result = crate::tools::binance_perps::close_position(creds, symbol)
            .await
            .map_err(|e| e.to_string());
    } else if prev_side == "flat" {
        let side = if new_side == "long" { "BUY" } else { "SELL" };
        side_str = side.to_lowercase();
        tracing::info!("[BN LIVE] Opening {new_side} {symbol} @ ~{price:.2} | {exec_size_usd:.2} USD ({qty:.6})");
        order_result = crate::tools::binance_perps::place_market_order(creds, symbol, side, qty, false)
            .await
            .map_err(|e| e.to_string());
    } else if prev_side != new_side {
        side_str = format!("reverse_{}_to_{}", prev_side, new_side);
        tracing::info!("[BN LIVE] Reversing {prev_side} → {new_side} for {symbol}");
        let _ = crate::tools::binance_perps::close_position(creds, symbol).await;
        let side = if new_side == "long" { "BUY" } else { "SELL" };
        order_result = crate::tools::binance_perps::place_market_order(creds, symbol, side, qty, false)
            .await
            .map_err(|e| e.to_string());
    } else {
        return;
    }

    let timestamp = chrono::Utc::now().to_rfc3339();
    match order_result {
        Ok(resp) => {
            let oid = resp.get("orderId").and_then(|v| v.as_u64()).map(|id| id.to_string()).unwrap_or_else(|| "ok".to_string());
            tracing::info!("[BN LIVE] Order placed: {oid}");
            gate.record_fill(&risk_manager::general::FillRecord {
                symbol: symbol.to_string(),
                strategy_id: config.id.clone(),
                side: side_str.clone(),
                size_usd: exec_size_usd,
                price,
                pnl_realized: 0.0,
                is_memecoin: false,
                timestamp: chrono::Utc::now(),
            });
            live_orders.push(LiveOrder {
                timestamp,
                window_ts: 0,
                side: side_str,
                token_id: symbol.to_string(),
                amount_usdc: exec_size_usd,
                order_id: oid,
                status: "filled".to_string(),
                entry_price: Some(price),
                result: None,
                pnl: None,
                stop_loss_triggered: false,
                ..Default::default()
            });
        }
        Err(e) => {
            tracing::error!("[BN LIVE] Order failed: {e}");
            live_orders.push(LiveOrder {
                timestamp,
                window_ts: 0,
                side: side_str,
                token_id: symbol.to_string(),
                amount_usdc: exec_size_usd,
                order_id: "FAILED".to_string(),
                status: format!("error: {e}"),
                entry_price: Some(price),
                result: None,
                pnl: None,
                stop_loss_triggered: false,
                ..Default::default()
            });
        }
    }
}

// ── Forced open/close helpers (funding arb) ──────────────────────────────────

async fn open_hl_long(
    client: &hyperliquid_trader::HyperliquidClient,
    gate: &risk_manager::general::TradingRiskGate,
    coin: &str,
    size_usd: f64,
    price: f64,
    config: &RunnerConfig,
    live_orders: &mut Vec<LiveOrder>,
) {
    if size_usd <= 0.0 {
        return;
    }
    let order_req = risk_manager::general::OrderRequest {
        symbol: coin.to_string(),
        strategy_id: config.id.clone(),
        side: "buy".into(),
        proposed_size_usd: size_usd,
        stop_distance_atr: 2.0,
        atr14: 0.0,
        is_memecoin: false,
    };
    let ctx = risk_manager::general::OrderContext { current_price: price };
    let approved = match gate.approve_order(&order_req, &ctx) {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!("[HL_ARB] Risk gate rejected long: {e}");
            return;
        }
    };
    let sz = approved.approved_size_usd / price;
    let order = hyperliquid_trader::types::Order::market_buy(coin, sz);
    let ts = chrono::Utc::now().to_rfc3339();
    match client.place_order(&order).await {
        Ok(resp) => {
            let oid = resp.order_id.map(|id| id.to_string()).unwrap_or_else(|| "ok".to_string());
            tracing::info!("[HL_ARB] Long {coin} @ ~{price:.2} | {sz:.6} coins");
            gate.record_fill(&risk_manager::general::FillRecord {
                symbol: coin.to_string(),
                strategy_id: config.id.clone(),
                side: "buy".into(),
                size_usd: approved.approved_size_usd,
                price,
                pnl_realized: 0.0,
                is_memecoin: false,
                timestamp: chrono::Utc::now(),
            });
            live_orders.push(LiveOrder {
                timestamp: ts,
                window_ts: 0,
                side: "buy".to_string(),
                token_id: coin.to_string(),
                amount_usdc: approved.approved_size_usd,
                order_id: oid,
                status: "filled".to_string(),
                entry_price: Some(price),
                result: None,
                pnl: None,
                stop_loss_triggered: false,
                ..Default::default()
            });
        }
        Err(e) => {
            tracing::error!("[HL_ARB] Long failed: {e}");
            live_orders.push(LiveOrder {
                timestamp: ts,
                window_ts: 0,
                side: "buy".to_string(),
                token_id: coin.to_string(),
                amount_usdc: size_usd,
                order_id: "FAILED".to_string(),
                status: format!("error: {e}"),
                entry_price: Some(price),
                result: None,
                pnl: None,
                stop_loss_triggered: false,
                ..Default::default()
            });
        }
    }
}

async fn open_hl_short(
    client: &hyperliquid_trader::HyperliquidClient,
    gate: &risk_manager::general::TradingRiskGate,
    coin: &str,
    size_usd: f64,
    price: f64,
    config: &RunnerConfig,
    live_orders: &mut Vec<LiveOrder>,
) {
    if size_usd <= 0.0 {
        return;
    }
    let order_req = risk_manager::general::OrderRequest {
        symbol: coin.to_string(),
        strategy_id: config.id.clone(),
        side: "sell".into(),
        proposed_size_usd: size_usd,
        stop_distance_atr: 2.0,
        atr14: 0.0,
        is_memecoin: false,
    };
    let ctx = risk_manager::general::OrderContext { current_price: price };
    let approved = match gate.approve_order(&order_req, &ctx) {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!("[HL_ARB] Risk gate rejected short: {e}");
            return;
        }
    };
    let sz = approved.approved_size_usd / price;
    let order = hyperliquid_trader::types::Order::market_sell(coin, sz);
    let ts = chrono::Utc::now().to_rfc3339();
    match client.place_order(&order).await {
        Ok(resp) => {
            let oid = resp.order_id.map(|id| id.to_string()).unwrap_or_else(|| "ok".to_string());
            tracing::info!("[HL_ARB] Short {coin} @ ~{price:.2} | {sz:.6} coins");
            gate.record_fill(&risk_manager::general::FillRecord {
                symbol: coin.to_string(),
                strategy_id: config.id.clone(),
                side: "sell".into(),
                size_usd: approved.approved_size_usd,
                price,
                pnl_realized: 0.0,
                is_memecoin: false,
                timestamp: chrono::Utc::now(),
            });
            live_orders.push(LiveOrder {
                timestamp: ts,
                window_ts: 0,
                side: "sell".to_string(),
                token_id: coin.to_string(),
                amount_usdc: approved.approved_size_usd,
                order_id: oid,
                status: "filled".to_string(),
                entry_price: Some(price),
                result: None,
                pnl: None,
                stop_loss_triggered: false,
                ..Default::default()
            });
        }
        Err(e) => {
            tracing::error!("[HL_ARB] Short failed: {e}");
            live_orders.push(LiveOrder {
                timestamp: ts,
                window_ts: 0,
                side: "sell".to_string(),
                token_id: coin.to_string(),
                amount_usdc: size_usd,
                order_id: "FAILED".to_string(),
                status: format!("error: {e}"),
                entry_price: Some(price),
                result: None,
                pnl: None,
                stop_loss_triggered: false,
                ..Default::default()
            });
        }
    }
}

async fn close_hl_position(
    client: &hyperliquid_trader::HyperliquidClient,
    gate: &risk_manager::general::TradingRiskGate,
    coin: &str,
    config: &RunnerConfig,
    live_orders: &mut Vec<LiveOrder>,
) {
    let ts = chrono::Utc::now().to_rfc3339();
    match client.close_position(coin).await {
        Ok(resp) => {
            let oid = resp.order_id.map(|id| id.to_string()).unwrap_or_else(|| "ok".to_string());
            tracing::info!("[HL_ARB] Closed {coin}");
            gate.record_fill(&risk_manager::general::FillRecord {
                symbol: coin.to_string(),
                strategy_id: config.id.clone(),
                side: "close".into(),
                size_usd: 0.0,
                price: 0.0,
                pnl_realized: 0.0,
                is_memecoin: false,
                timestamp: chrono::Utc::now(),
            });
            live_orders.push(LiveOrder {
                timestamp: ts,
                window_ts: 0,
                side: "close".to_string(),
                token_id: coin.to_string(),
                amount_usdc: 0.0,
                order_id: oid,
                status: "filled".to_string(),
                entry_price: None,
                result: None,
                pnl: None,
                stop_loss_triggered: false,
                ..Default::default()
            });
        }
        Err(e) => {
            tracing::error!("[HL_ARB] Close failed: {e}");
            live_orders.push(LiveOrder {
                timestamp: ts,
                window_ts: 0,
                side: "close".to_string(),
                token_id: coin.to_string(),
                amount_usdc: 0.0,
                order_id: "FAILED".to_string(),
                status: format!("error: {e}"),
                entry_price: None,
                result: None,
                pnl: None,
                stop_loss_triggered: false,
                ..Default::default()
            });
        }
    }
}

async fn open_binance_long(
    creds: &crate::tools::binance_perps::BinanceCredentials,
    gate: &risk_manager::general::TradingRiskGate,
    symbol: &str,
    size_usd: f64,
    price: f64,
    config: &RunnerConfig,
    live_orders: &mut Vec<LiveOrder>,
) {
    if size_usd <= 0.0 {
        return;
    }
    let order_req = risk_manager::general::OrderRequest {
        symbol: symbol.to_string(),
        strategy_id: config.id.clone(),
        side: "buy".into(),
        proposed_size_usd: size_usd,
        stop_distance_atr: 2.0,
        atr14: 0.0,
        is_memecoin: false,
    };
    let ctx = risk_manager::general::OrderContext { current_price: price };
    let approved = match gate.approve_order(&order_req, &ctx) {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!("[BN_ARB] Risk gate rejected long: {e}");
            return;
        }
    };
    let qty = approved.approved_size_usd / price;
    let ts = chrono::Utc::now().to_rfc3339();
    match crate::tools::binance_perps::place_market_order(creds, symbol, "BUY", qty, false).await {
        Ok(resp) => {
            let oid = resp.get("orderId").and_then(|v| v.as_u64()).map(|id| id.to_string()).unwrap_or_else(|| "ok".to_string());
            tracing::info!("[BN_ARB] Long {symbol} @ ~{price:.2} | {qty:.6}");
            gate.record_fill(&risk_manager::general::FillRecord {
                symbol: symbol.to_string(),
                strategy_id: config.id.clone(),
                side: "buy".into(),
                size_usd: approved.approved_size_usd,
                price,
                pnl_realized: 0.0,
                is_memecoin: false,
                timestamp: chrono::Utc::now(),
            });
            live_orders.push(LiveOrder {
                timestamp: ts,
                window_ts: 0,
                side: "buy".to_string(),
                token_id: symbol.to_string(),
                amount_usdc: approved.approved_size_usd,
                order_id: oid,
                status: "filled".to_string(),
                entry_price: Some(price),
                result: None,
                pnl: None,
                stop_loss_triggered: false,
                ..Default::default()
            });
        }
        Err(e) => {
            tracing::error!("[BN_ARB] Long failed: {e}");
            live_orders.push(LiveOrder {
                timestamp: ts,
                window_ts: 0,
                side: "buy".to_string(),
                token_id: symbol.to_string(),
                amount_usdc: size_usd,
                order_id: "FAILED".to_string(),
                status: format!("error: {e}"),
                entry_price: Some(price),
                result: None,
                pnl: None,
                stop_loss_triggered: false,
                ..Default::default()
            });
        }
    }
}

async fn open_binance_short(
    creds: &crate::tools::binance_perps::BinanceCredentials,
    gate: &risk_manager::general::TradingRiskGate,
    symbol: &str,
    size_usd: f64,
    price: f64,
    config: &RunnerConfig,
    live_orders: &mut Vec<LiveOrder>,
) {
    if size_usd <= 0.0 {
        return;
    }
    let order_req = risk_manager::general::OrderRequest {
        symbol: symbol.to_string(),
        strategy_id: config.id.clone(),
        side: "sell".into(),
        proposed_size_usd: size_usd,
        stop_distance_atr: 2.0,
        atr14: 0.0,
        is_memecoin: false,
    };
    let ctx = risk_manager::general::OrderContext { current_price: price };
    let approved = match gate.approve_order(&order_req, &ctx) {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!("[BN_ARB] Risk gate rejected short: {e}");
            return;
        }
    };
    let qty = approved.approved_size_usd / price;
    let ts = chrono::Utc::now().to_rfc3339();
    match crate::tools::binance_perps::place_market_order(creds, symbol, "SELL", qty, false).await {
        Ok(resp) => {
            let oid = resp.get("orderId").and_then(|v| v.as_u64()).map(|id| id.to_string()).unwrap_or_else(|| "ok".to_string());
            tracing::info!("[BN_ARB] Short {symbol} @ ~{price:.2} | {qty:.6}");
            gate.record_fill(&risk_manager::general::FillRecord {
                symbol: symbol.to_string(),
                strategy_id: config.id.clone(),
                side: "sell".into(),
                size_usd: approved.approved_size_usd,
                price,
                pnl_realized: 0.0,
                is_memecoin: false,
                timestamp: chrono::Utc::now(),
            });
            live_orders.push(LiveOrder {
                timestamp: ts,
                window_ts: 0,
                side: "sell".to_string(),
                token_id: symbol.to_string(),
                amount_usdc: approved.approved_size_usd,
                order_id: oid,
                status: "filled".to_string(),
                entry_price: Some(price),
                result: None,
                pnl: None,
                stop_loss_triggered: false,
                ..Default::default()
            });
        }
        Err(e) => {
            tracing::error!("[BN_ARB] Short failed: {e}");
            live_orders.push(LiveOrder {
                timestamp: ts,
                window_ts: 0,
                side: "sell".to_string(),
                token_id: symbol.to_string(),
                amount_usdc: size_usd,
                order_id: "FAILED".to_string(),
                status: format!("error: {e}"),
                entry_price: Some(price),
                result: None,
                pnl: None,
                stop_loss_triggered: false,
                ..Default::default()
            });
        }
    }
}

async fn close_binance_position(
    creds: &crate::tools::binance_perps::BinanceCredentials,
    gate: &risk_manager::general::TradingRiskGate,
    symbol: &str,
    config: &RunnerConfig,
    live_orders: &mut Vec<LiveOrder>,
) {
    let ts = chrono::Utc::now().to_rfc3339();
    match crate::tools::binance_perps::close_position(creds, symbol).await {
        Ok(resp) => {
            let oid = resp.get("orderId").and_then(|v| v.as_u64()).map(|id| id.to_string()).unwrap_or_else(|| "ok".to_string());
            tracing::info!("[BN_ARB] Closed {symbol}");
            gate.record_fill(&risk_manager::general::FillRecord {
                symbol: symbol.to_string(),
                strategy_id: config.id.clone(),
                side: "close".into(),
                size_usd: 0.0,
                price: 0.0,
                pnl_realized: 0.0,
                is_memecoin: false,
                timestamp: chrono::Utc::now(),
            });
            live_orders.push(LiveOrder {
                timestamp: ts,
                window_ts: 0,
                side: "close".to_string(),
                token_id: symbol.to_string(),
                amount_usdc: 0.0,
                order_id: oid,
                status: "filled".to_string(),
                entry_price: None,
                result: None,
                pnl: None,
                stop_loss_triggered: false,
                ..Default::default()
            });
        }
        Err(e) => {
            tracing::error!("[BN_ARB] Close failed: {e}");
            live_orders.push(LiveOrder {
                timestamp: ts,
                window_ts: 0,
                side: "close".to_string(),
                token_id: symbol.to_string(),
                amount_usdc: 0.0,
                order_id: "FAILED".to_string(),
                status: format!("error: {e}"),
                entry_price: None,
                result: None,
                pnl: None,
                stop_loss_triggered: false,
                ..Default::default()
            });
        }
    }
}

async fn crypto_runner_loop(
    store: Arc<StrategyRunnerStore>,
    config: RunnerConfig,
    workspace_dir: PathBuf,
) {
    let id = config.id.clone();
    let is_live = config.mode == "live";

    // ─ Live trading setup ──────────────────────────────────────────────────────
    let hl_client: Option<hyperliquid_trader::HyperliquidClient> = if is_live {
        if let Some(ref signer) = config.hl_signer {
            let client = hyperliquid_trader::HyperliquidClient::new_mainnet_with_signer(signer.clone());
            tracing::info!("[RUNNER {id}] Live trading enabled on Hyperliquid (addr={})", signer.address());
            Some(client)
        } else {
            None
        }
    } else {
        None
    };

    let use_binance = is_live && config.binance_creds.is_some() && config.hl_signer.is_none();
    if use_binance {
        tracing::info!("[RUNNER {id}] Live trading enabled on Binance Futures");
    } else if is_live && hl_client.is_none() && config.hl_signer.is_none() {
        tracing::warn!("[RUNNER {id}] Live mode requested but no trading venue configured; running paper");
    }

    let risk_gate = config.risk_gate.clone();
    let coin = binance_symbol_to_hl_coin(&config.symbol);

    // ─ Resolve script (read from disk or fall back to bundled default)
    let script_content = match crate::tools::backtest::read_script_or_default(
        &workspace_dir, &config.script) {
        Some(s) => s,
        None => {
            set_runner_error(&store, &id, &format!("Script not found: {}", config.script));
            return;
        }
    };

    // ─ Warmup: fetch recent candles from Binance REST ──────────────────────────
    // We need enough history for indicators (RSI-14, EMA-200 → 500 candles is safe).
    let warmup_limit: usize = {
        let candles_per_day = (86_400 / interval_to_secs(&config.interval).max(60)) as usize;
        (config.warmup_days.max(1) as usize * candles_per_day).clamp(100, 1000)
    };
    tracing::info!("[RUNNER {id}] Fetching {warmup_limit} warmup candles for {}@{}", config.symbol, config.interval);

    let warmup = match crate::tools::backtest::fetch_recent_candles(
        &config.symbol, &config.interval, warmup_limit,
    ).await {
        Ok(c) => c,
        Err(e) => {
            set_runner_error(&store, &id, &format!("Warmup fetch failed: {e}"));
            return;
        }
    };
    tracing::info!("[RUNNER {id}] Warmup: {} candles loaded", warmup.len());

    // ─ Rolling candle buffer (max 1000 to keep indicator quality high) ─────────
    const MAX_BUFFER: usize = 1000;
    let mut buffer: VecDeque<crate::tools::backtest::Candle> = warmup.into_iter().collect();

    // Initial evaluation on warmup data
    let initial_metrics = crate::tools::backtest::run_rhai_on_candle_buffer(
        &script_content,
        buffer.iter().cloned().collect(),
        config.initial_balance,
        config.fee_pct,
    );
    let mut last_position = initial_metrics.position;
    // Rehydrate live_orders from the persisted store so paper/live trade
    // history survives pause/restart cycles. Mirror what
    // `polymarket_runner_loop` does at startup.
    let mut live_orders: Vec<LiveOrder> = {
        let map = store.runners.lock().unwrap();
        map.get(&id)
            .and_then(|r| r.result.as_ref().map(|res| res.live_orders.clone()))
            .unwrap_or_default()
    };
    if !live_orders.is_empty() {
        tracing::info!(
            "[RUNNER {id}] Restored {} live_orders from persisted state",
            live_orders.len()
        );
    }
    update_runner_result(&store, &id, &config, &initial_metrics, None, None, None, None, None, None, None).await;
    set_runner_status(&store, &id, "running");

    // ─ Connect Binance WebSocket for real-time closed candles ──────────────────
    let mut candle_rx = crate::live_feed::spawn_binance_kline_feed(
        config.symbol.clone(),
        config.interval.clone(),
    );
    tracing::info!("[RUNNER {id}] Live feed started for {}@{}", config.symbol, config.interval);

    // ─ Main loop: process live candles as they close ───────────────────────────
    while let Some(live) = candle_rx.recv().await {
        let candle = crate::tools::backtest::Candle {
            open_time_ms: live.open_time_ms,
            open:   live.open,
            high:   live.high,
            low:    live.low,
            close:  live.close,
            volume: live.volume,
        };
        tracing::debug!("[RUNNER {id}] New closed candle: close={}", candle.close);

        buffer.push_back(candle.clone());
        if buffer.len() > MAX_BUFFER { buffer.pop_front(); }

        // Pre-flight risk gate check (live only)
        if is_live {
            if let Some(ref gate) = risk_gate {
                if gate.is_halted() {
                    tracing::warn!("[RUNNER {id}] Risk gate is HALTED — skipping tick");
                    let now = chrono::Utc::now().to_rfc3339();
                    {
                        let mut map = store.runners.lock().unwrap();
                        if let Some(r) = map.get_mut(&id) {
                            r.status.last_tick_at = Some(now);
                        }
                    }
                    continue;
                }
            }
        }

        // Run strategy on the current rolling window
        let metrics = crate::tools::backtest::run_rhai_on_candle_buffer(
            &script_content,
            buffer.iter().cloned().collect(),
            config.initial_balance,
            config.fee_pct,
        );

        // Live order execution (CEX)
        if is_live {
            let atr14 = compute_atr_simple(&buffer.iter().cloned().collect::<Vec<_>>(), 14);
            if let Some(ref gate) = risk_gate {
                if let Some(ref client) = hl_client {
                    execute_hl_position_change(
                        client, gate, &config, &coin, candle.close,
                        last_position, metrics.position, atr14,
                        &mut live_orders,
                    ).await;
                } else if let Some(ref creds) = config.binance_creds {
                    execute_binance_position_change(
                        creds, gate, &config, &config.symbol, candle.close,
                        last_position, metrics.position, atr14,
                        &mut live_orders,
                    ).await;
                }
            }
            last_position = metrics.position;
        }

        // Update status timestamps
        let now = chrono::Utc::now().to_rfc3339();
        {
            let mut map = store.runners.lock().unwrap();
            if let Some(r) = map.get_mut(&id) {
                r.status.last_tick_at = Some(now);
                r.status.next_tick_at = None; // event-driven; no fixed next tick
            }
        }
        update_runner_result(&store, &id, &config, &metrics, None, None, None, Some(live_orders.clone()), None, None, None).await;
        store.persist();
    }

    // Channel closed means the feed task was dropped (runner stopped)
    tracing::info!("[RUNNER {id}] Feed channel closed, exiting");
}

// ── Funding rate arbitrage runner ─────────────────────────────────────────────

/// Per-pair position tracking for funding arbitrage.
#[derive(Debug, Clone)]
struct FundingArbPosition {
    pub symbol: String,
    pub hl_side: String,      // "long" | "short" | "flat"
    pub bin_side: String,     // "long" | "short" | "flat"
    pub leg_size_usd: f64,
    pub entry_diff_apr: f64,
    pub opened_at: chrono::DateTime<chrono::Utc>,
}

async fn funding_arb_runner_loop(
    store: Arc<StrategyRunnerStore>,
    config: RunnerConfig,
    _workspace_dir: PathBuf,
) {
    let id = config.id.clone();
    let is_live = config.mode == "live";

    // ─ Live trading setup ──────────────────────────────────────────────────────
    let hl_client: Option<hyperliquid_trader::HyperliquidClient> = if is_live {
        config.hl_signer.as_ref().map(|s| {
            hyperliquid_trader::HyperliquidClient::new_mainnet_with_signer(s.clone())
        })
    } else {
        None
    };

    let binance_creds = config.binance_creds.clone();

    if is_live {
        if hl_client.is_none() {
            set_runner_error(&store, &id, "Live funding arb requires Hyperliquid wallet (hl_signer).");
            return;
        }
        if binance_creds.is_none() {
            set_runner_error(&store, &id, "Live funding arb requires Binance Futures credentials.");
            return;
        }
        tracing::info!("[RUNNER {id}] Live funding arb enabled on HL + Binance");
    } else {
        tracing::info!("[RUNNER {id}] Paper funding arb mode");
    }

    let risk_gate = config.risk_gate.clone();
    let watchlist = config.funding_watchlist.clone();
    let min_apr_diff = config.min_apr_diff;
    let force_close_diff = config.force_close_diff;
    let max_open_pairs = config.max_open_pairs;
    let max_pos_pct = config.max_pos_pct;
    let poll_secs = config.funding_poll_secs;
    let fee_buffer_bps = config.fee_buffer_bps;
    let fee_buffer_apr = (fee_buffer_bps / 10000.0) * 24.0 * 365.0 / 8.0;

    let mut open_positions: Vec<FundingArbPosition> = Vec::new();
    let mut live_orders: Vec<LiveOrder> = Vec::new();

    set_runner_status(&store, &id, "running");

    // ─ Main loop ───────────────────────────────────────────────────────────────
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(poll_secs));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        interval.tick().await;

        // Pre-flight risk gate check
        if is_live {
            if let Some(ref gate) = risk_gate {
                if gate.is_halted() {
                    tracing::warn!("[RUNNER {id}] Risk gate HALTED — skipping funding check");
                    let now = chrono::Utc::now().to_rfc3339();
                    {
                        let mut map = store.runners.lock().unwrap();
                        if let Some(r) = map.get_mut(&id) {
                            r.status.last_tick_at = Some(now);
                        }
                    }
                    continue;
                }
            }
        }

        // ─ Fetch Hyperliquid predicted funding ─────────────────────────────────
        let hl_rates: std::collections::HashMap<String, f64> = if let Some(ref client) = hl_client {
            match client.predicted_funding().await {
                Ok(rates) => rates,
                Err(e) => {
                    tracing::warn!("[RUNNER {id}] HL predicted funding failed: {e}");
                    std::collections::HashMap::new()
                }
            }
        } else {
            // Paper mode: simulate random funding rates for testing
            let mut sim = std::collections::HashMap::new();
            for coin in &watchlist {
                sim.insert(coin.clone(), (rand::random::<f64>() - 0.5) * 0.002);
            }
            sim
        };

        // ─ Fetch Binance funding rates ─────────────────────────────────────────
        let binance_rates: std::collections::HashMap<String, f64> =
            if let Some(ref creds) = binance_creds {
                match fetch_binance_funding_rates(creds, &watchlist).await {
                    Ok(rates) => rates,
                    Err(e) => {
                        tracing::warn!("[RUNNER {id}] Binance funding fetch failed: {e}");
                        std::collections::HashMap::new()
                    }
                }
            } else {
                let mut sim = std::collections::HashMap::new();
                for coin in &watchlist {
                    sim.insert(format!("{}USDT", coin), (rand::random::<f64>() - 0.5) * 0.001);
                }
                sim
            };

        // ─ Evaluate each symbol ────────────────────────────────────────────────
        let mut decisions: Vec<String> = Vec::new();
        for coin in &watchlist {
            let hl_raw = hl_rates.get(coin).copied().unwrap_or(0.0);
            let bin_raw = binance_rates.get(&format!("{}USDT", coin)).copied().unwrap_or(0.0);

            // HL is 1h funding, Binance is 8h funding
            let hl_apr = hl_raw * 24.0 * 365.0;
            let bin_apr = bin_raw * 3.0 * 365.0;
            let raw_diff = (hl_apr - bin_apr).abs();
            let net_diff = raw_diff - fee_buffer_apr;

            let pos_idx = open_positions.iter().position(|p| p.symbol == *coin);
            let in_pair = pos_idx.is_some();

            if !in_pair && net_diff > min_apr_diff && open_positions.len() < max_open_pairs {
                let capital = risk_gate.as_ref().map(|g| g.status().total_capital).unwrap_or(config.initial_balance);
                let leg_size_usd = capital * max_pos_pct * 0.5;

                let (hl_side, bin_side) = if hl_apr > bin_apr {
                    ("short", "long")
                } else {
                    ("long", "short")
                };

                if is_live {
                    if let (Some(ref client), Some(ref gate), Some(ref creds)) = (&hl_client, &risk_gate, &binance_creds) {
                        // Fetch reference price from Binance spot for sizing
                        let price = fetch_binance_price(coin).await.unwrap_or(0.0);
                        if price > 0.0 {
                            if hl_side == "long" {
                                open_hl_long(client, gate, coin, leg_size_usd, price, &config, &mut live_orders).await;
                            } else {
                                open_hl_short(client, gate, coin, leg_size_usd, price, &config, &mut live_orders).await;
                            }
                            let symbol = format!("{}USDT", coin);
                            if bin_side == "long" {
                                open_binance_long(creds, gate, &symbol, leg_size_usd, price, &config, &mut live_orders).await;
                            } else {
                                open_binance_short(creds, gate, &symbol, leg_size_usd, price, &config, &mut live_orders).await;
                            }
                        }
                    }
                } else {
                    tracing::info!("[RUNNER {id}] PAPER: Would open {coin} arb | HL={hl_side} BIN={bin_side} | net_diff={net_diff:.2}%");
                }

                open_positions.push(FundingArbPosition {
                    symbol: coin.clone(),
                    hl_side: hl_side.to_string(),
                    bin_side: bin_side.to_string(),
                    leg_size_usd,
                    entry_diff_apr: net_diff,
                    opened_at: chrono::Utc::now(),
                });
                decisions.push(format!("OPEN {coin} {hl_side}/{bin_side} @ {net_diff:.2}% APR"));

            } else if in_pair && raw_diff < force_close_diff {
                if let Some(idx) = pos_idx {
                    let pos = open_positions.remove(idx);
                    if is_live {
                        if let (Some(ref client), Some(ref gate), Some(ref creds)) = (&hl_client, &risk_gate, &binance_creds) {
                            close_hl_position(client, gate, coin, &config, &mut live_orders).await;
                            let symbol = format!("{}USDT", coin);
                            close_binance_position(creds, gate, &symbol, &config, &mut live_orders).await;
                        }
                    } else {
                        tracing::info!("[RUNNER {id}] PAPER: Would close {coin} arb | diff collapsed to {raw_diff:.2}%");
                    }
                    decisions.push(format!("CLOSE {coin} @ {raw_diff:.2}% APR"));
                }
            }
        }

        // ─ Update runner status ────────────────────────────────────────────────
        let now = chrono::Utc::now().to_rfc3339();
        let analysis = if decisions.is_empty() {
            "No funding arb signals this cycle".to_string()
        } else {
            decisions.join("; ")
        };

        // Build a minimal RunnerResult for funding arb
        let result = RunnerResult {
            total_return_pct: 0.0,
            balance: config.initial_balance,
            position: open_positions.len() as f64,
            total_trades: live_orders.len() as u32,
            win_rate_pct: 0.0,
            sharpe_ratio: 0.0,
            max_drawdown_pct: 0.0,
            all_trades: Vec::new(),
            last_signal: analysis.clone(),
            analysis,
            live_feed: None,
            wallet_address: config.wallet_address.clone(),
            wallet_balance_usdc: None,
            live_orders: live_orders.clone(),
            live_wins: 0,
            live_total_trades: live_orders.len() as u32,
            live_kv_state: std::collections::HashMap::new(),
        };

        {
            let mut map = store.runners.lock().unwrap();
            if let Some(r) = map.get_mut(&id) {
                r.result = Some(result);
                r.status.last_tick_at = Some(now);
                r.status.status = "running".to_string();
            }
        }
        store.persist();
    }
}

/// Fetch Binance funding rates for a list of coins via REST.
async fn fetch_binance_funding_rates(
    _creds: &crate::tools::binance_perps::BinanceCredentials,
    watchlist: &[String],
) -> anyhow::Result<std::collections::HashMap<String, f64>> {
    let client = reqwest::Client::new();
    let url = "https://fapi.binance.com/fapi/v1/premiumIndex";
    let resp = client.get(url).send().await?;
    let arr: Vec<serde_json::Value> = resp.json().await?;

    let mut rates: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    for item in &arr {
        let symbol = item["symbol"].as_str().unwrap_or("");
        let rate_str = item["lastFundingRate"].as_str().unwrap_or("0");
        let rate: f64 = rate_str.parse().unwrap_or(0.0);
        for coin in watchlist {
            if symbol == format!("{}USDT", coin) {
                rates.insert(symbol.to_string(), rate);
            }
        }
    }
    Ok(rates)
}

/// Fetch current Binance spot price for a coin.
async fn fetch_binance_price(coin: &str) -> Option<f64> {
    let client = reqwest::Client::new();
    let url = format!("https://api.binance.com/api/v3/ticker/price?symbol={}USDT", coin);
    match client.get(&url).send().await {
        Ok(resp) => match resp.json::<serde_json::Value>().await {
            Ok(json) => json["price"].as_str().and_then(|s| s.parse().ok()),
            Err(_) => None,
        },
        Err(_) => None,
    }
}

// ── Polymarket binary live runner ─────────────────────────────────────────────
//
// For polymarket_binary, each window (5m/15m/1h/24h) is a separate market.
/// Query data-api.polymarket.com for the last 48h of on-chain activity for
/// `wallet_address`. Any TRADE transaction whose hash is not present in the
/// runner's live_orders is inserted as an UNTRACKED order so the dashboard
/// shows the real P&L instead of only the in-memory log.
async fn reconcile_untracked_onchain(
    store: &Arc<StrategyRunnerStore>,
    id: &str,
    wallet_address: &str,
) {
    let url = format!(
        "https://data-api.polymarket.com/activity?user={}&limit=500&offset=0",
        wallet_address
    );
    let client = reqwest::Client::builder()
        .user_agent("trader-claw/1.0")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap_or_default();

    let onchain: Vec<serde_json::Value> = match client.get(&url).send().await {
        Ok(r) if r.status().is_success() => {
            r.json().await.unwrap_or_default()
        }
        _ => {
            tracing::debug!("[RUNNER {id}] Onchain reconciliation: data-api unavailable, skipping");
            return;
        }
    };

    // Filter to TRADE events in the last 48 h
    let cutoff = chrono::Utc::now().timestamp() - 48 * 3600;
    let trades: Vec<&serde_json::Value> = onchain.iter()
        .filter(|t| {
            t.get("type").and_then(|v| v.as_str()) == Some("TRADE")
            && t.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0) >= cutoff
        })
        .collect();

    if trades.is_empty() { return; }

    // Collect known tx hashes from existing runner orders
    let known_tx: std::collections::HashSet<String> = {
        let map = store.runners.lock().unwrap();
        map.get(id)
            .and_then(|r| r.result.as_ref())
            .map(|res| res.live_orders.iter()
                .filter_map(|o| o.tx_hash.clone())
                .map(|h| h.to_lowercase())
                .collect())
            .unwrap_or_default()
    };

    let mut new_orders: Vec<LiveOrder> = Vec::new();
    for t in &trades {
        let tx = t.get("transactionHash")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        if tx.is_empty() || known_tx.contains(&tx) { continue; }

        let ts_unix = t.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0);
        let ts_str = chrono::DateTime::from_timestamp(ts_unix, 0)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default();
        let usdc = t.get("usdcSize").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let size = t.get("size").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let ep = if size > 0.0 && usdc > 0.0 { usdc / size } else { 0.0 };
        let outcome = t.get("outcome").and_then(|v| v.as_str()).unwrap_or("?").to_lowercase();
        let side = if outcome.contains("up") || outcome.contains("yes") { "yes" } else { "no" };

        new_orders.push(LiveOrder {
            timestamp: ts_str,
            window_ts: ts_unix - (ts_unix % 300),
            side: side.to_string(),
            token_id: t.get("conditionId").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            amount_usdc: usdc,
            order_id: format!("onchain-{}", &tx[..tx.len().min(16)]),
            status: "MATCHED".to_string(),
            entry_price: if ep > 0.0 { Some(ep) } else { None },
            tx_hash: Some(tx),
            result: Some("UNTRACKED".to_string()),
            ..Default::default()
        });
    }

    if new_orders.is_empty() { return; }

    let count = new_orders.len();
    {
        let mut map = store.runners.lock().unwrap();
        if let Some(r) = map.get_mut(id) {
            if let Some(ref mut res) = r.result {
                res.live_orders.extend(new_orders);
            }
        }
    }
    store.persist();
    append_runner_log(store, id, &format!(
        "Onchain reconciliation: {} untracked transaction(s) added from data-api",
        count
    ));
    tracing::info!("[RUNNER {id}] Reconciled {count} untracked onchain transactions");
}

// Paper mode: uses a 1m Binance WebSocket feed and re-runs full slug simulation
//             at each window boundary to generate a fresh decision.
// Live mode:  same signal generation, but then calls Polymarket CLOB API to
//             place/cancel real orders. Verifies credentials and USDC/pUSD balance first.

async fn polymarket_runner_loop(
    store: Arc<StrategyRunnerStore>,
    mut config: RunnerConfig,
    workspace_dir: PathBuf,
    config_path: Option<PathBuf>,
) {
    let id = config.id.clone();
    let is_live = config.mode == "live";
    let window_secs = interval_to_secs(&config.interval).max(60);
    let window_minutes = (window_secs / 60).max(1) as usize;

    // ─ Resolve script (read from disk or fall back to bundled default)
    let script_content = match crate::tools::backtest::read_script_or_default(&workspace_dir, &config.script) {
        Some(s) => s,
        None => {
            set_runner_error(&store, &id, &format!("Script not found: {}", config.script));
            return;
        }
    };

    // ─ Live mode: validate credentials and build CLOB client.
    //
    // Token IDs are resolved dynamically per window in the main loop (see
    // `resolve_token_for_window` at the `current_window != last_window`
    // boundary). We do NOT require a token to be pre-populated on entry:
    //   - On first window, `last_window = -1` triggers resolve immediately.
    //   - On manual restart after a stop, the persisted config has
    //     `poly_token_id = None` (it's #[serde(skip)]) and the rehydrate path
    //     may not populate it if the slug for the current minute hasn't been
    //     listed yet by Polymarket (transient).
    // Failing here would leave the runner stuck in error until manually
    // recreated. Instead we proceed and let the per-window resolver retry
    // with backoff every minute until the slug appears.
    let mut clob_client: Option<std::sync::Arc<polymarket_trader::orders::ClobClient>> = if is_live {
        if config.series_id.as_deref().unwrap_or("").trim().is_empty() {
            set_runner_error(
                &store,
                &id,
                "Live mode cannot start: no Market Series selected. Recreate the strategy and choose a supported series (BTC/ETH/SOL/...).",
            );
            return;
        }

        match &config.poly_creds {
            None => {
                set_runner_error(
                    &store, &id,
                    "Live mode requires Polymarket credentials. Set api_key, secret, and passphrase in Settings → Config → [polymarket].",
                );
                return;
            }
            Some(creds) => {
                if creds.api_key.is_empty() || creds.secret.is_empty() || creds.passphrase.is_empty() {
                    set_runner_error(
                        &store, &id,
                        "Live mode: Polymarket credentials incomplete. Check api_key, secret, and passphrase in Settings → Config.",
                    );
                    return;
                }
                let client = std::sync::Arc::new(
                    polymarket_trader::orders::ClobClient::new(creds.clone())
                );
                tracing::info!("[RUNNER {id}] Live mode: CLOB client created, api_key={}...", &creds.api_key[..8.min(creds.api_key.len())]);
                Some(client)
            }
        }
    } else {
        None
    };

    // ─ Warmup: fetch recent 1m candles (polymarket binary always uses 1m)
    let warmup_limit: usize = {
        let candles_per_day = 1440_usize; // 1m candles
        (config.warmup_days.max(1) as usize * candles_per_day).clamp(200, 1000)
    };
    // Resolve the underlying Binance pair (config.symbol might be a Polymarket slug)
    let binance_sym = binance_symbol_for_polymarket(&config.symbol);
    tracing::info!("[RUNNER {id}] Polymarket warmup: {warmup_limit} x 1m candles for {binance_sym}@{}", config.interval);

    let warmup = match crate::tools::backtest::fetch_recent_candles(
        &binance_sym, "1m", warmup_limit,
    ).await {
        Ok(c) => c,
        Err(e) => { set_runner_error(&store, &id, &format!("Warmup failed: {e}")); return; }
    };
    tracing::info!("[RUNNER {id}] Warmup: {} 1m candles loaded", warmup.len());

    const MAX_BUFFER: usize = 2000; // ~33h of 1m candles
    let mut buffer: std::collections::VecDeque<crate::tools::backtest::Candle> =
        warmup.into_iter().collect();

    // Initial evaluation on warmup data
    let initial = eval_polymarket(&script_content, &buffer, window_minutes, &config);
    update_runner_result(&store, &id, &config, &initial, None, None, None, None, None, None, None).await;
    set_runner_status(&store, &id, "running");

    // ─ Onchain reconciliation (live mode): patch any untracked transactions ────
    // Query data-api.polymarket.com for the last 48h of activity on the proxy
    // wallet. Transactions that exist onchain but are absent from live_orders
    // (e.g. because the binary crashed before flushing to disk) are inserted
    // as UNTRACKED records so the dashboard reflects the real P&L.
    if is_live {
        if let Some(ref creds) = config.poly_creds {
            let wallet = creds.proxy_address.clone()
                .or_else(|| Some(creds.wallet_address.clone()))
                .unwrap_or_default();
            if !wallet.is_empty() {
                let store_rc = store.clone();
                let id_rc    = id.clone();
                let wallet_rc = wallet.clone();
                tokio::spawn(async move {
                    reconcile_untracked_onchain(&store_rc, &id_rc, &wallet_rc).await;
                });
            }
        }
    }

    // ─ Connect 1m WebSocket (polymarket binary always uses 1m real-time feed)
    let mut candle_rx = crate::live_feed::spawn_binance_kline_feed(
        binance_sym.clone(),
        "1m".to_string(),
    );
    tracing::info!("[RUNNER {id}] Polymarket live feed started: {binance_sym}@1m (window={window_secs}s)");

    // ─ Optional Chainlink price feed (overrides displayed BTC price)
    let chainlink_price: Option<crate::live_feed::ChainlinkPriceHandle> =
        config.chainlink_endpoint_url.as_ref().map(|url| {
            tracing::info!("[RUNNER {id}] Chainlink price feed enabled: {url}");
            crate::live_feed::spawn_chainlink_price_feed(
                url.clone(),
                config.chainlink_api_key.clone(),
                config.chainlink_interval_secs.max(1),
            )
        });

    // ─ Binance 1s miniTicker — updates displayed price every second
    // Also kept in a shared atomic for oracle-comparison injection into ctx.
    let latest_binance_price = std::sync::Arc::new(std::sync::RwLock::new(0f64));
    let lbp_write = latest_binance_price.clone();
    let mut ticker_rx = crate::live_feed::spawn_binance_ticker_feed(binance_sym.clone());
    let store_for_ticker = store.clone();
    let id_for_ticker = id.clone();
    tokio::spawn(async move {
        while let Some(price) = ticker_rx.recv().await {
            *lbp_write.write().unwrap() = price;
            let mut map = store_for_ticker.runners.lock().unwrap();
            if let Some(r) = map.get_mut(&id_for_ticker) {
                if let Some(ref mut result) = r.result {
                    if let Some(ref mut feed) = result.live_feed {
                        feed.current_btc_price = price;
                        // Also push to price_history for the mini chart (throttle to avoid overflow)
                        let now_ms = chrono::Utc::now().timestamp_millis();
                        if feed.price_history.len() >= 300 {
                            feed.price_history.remove(0);
                        }
                        feed.price_history.push((now_ms, price));
                    }
                }
            }
        }
    });

    // ─ 1-second wall-clock timer: update dashboard window state in real time ─
    // This prevents the dashboard from lagging 1 minute behind at window start,
    // since candles only arrive when they close (1m delay).
    let store_for_timer = store.clone();
    let id_for_timer = id.clone();
    let window_secs_for_timer = window_secs;
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            tick.tick().await;
            let now = chrono::Utc::now().timestamp();
            let current_window = now - (now % window_secs_for_timer as i64);
            let next_window = current_window + window_secs_for_timer as i64;
            let window_seconds_left = next_window - now;
            let mut map = store_for_timer.runners.lock().unwrap();
            if let Some(r) = map.get_mut(&id_for_timer) {
                if let Some(ref mut result) = r.result {
                    if let Some(ref mut feed) = result.live_feed {
                        feed.window_timestamp = current_window;
                        feed.window_seconds_left = window_seconds_left;
                    }
                }
            }
        }
    });

    let mut last_window: i64 = -1;
    let mut last_decision_window: i64 = -1;
    // Drift signal: YES token price captured at the candle BEFORE the decision
    // candle closes (60s pre-decision). Reset to 0.0 after each decision so a
    // stale value doesn't bleed into the next window. 0.0 = unavailable.
    let mut prev_yes_token_price: f64 = 0.0;
    let mut prev_yes_captured_window: i64 = -1;
    // (window_ts, live_signal, bt_preview_signal, debug)
    // bt_preview_signal is captured at decision time (stateless, no capital constraint)
    // and used for signal comparison so a depleted BT balance doesn't cause false DISCREPANCYs.
    let mut prev_live_position: Option<(i64, String, String, std::collections::HashMap<String, f64>)> = None;
    let mut price_history: std::collections::VecDeque<(i64, f64)> = std::collections::VecDeque::with_capacity(60);
    // Persistent kv state for live signal — carries avg_vol and other ctx.set() values across windows.
    //
    // On restart, we prefer the previously persisted kv_state from the store
    // so script state (loss_streak, pause_until, avg_vol, ...) survives a
    // pause/restart cycle. If there's no prior state (fresh runner), we seed
    // from a BT warmup run so avg_vol starts aligned with the backtester's
    // historical value, preventing score divergence on the first windows.
    let mut live_kv_state: std::collections::HashMap<String, f64> = {
        let restored = {
            let map = store.runners.lock().unwrap();
            map.get(&id)
                .and_then(|r| r.result.as_ref())
                .map(|res| res.live_kv_state.clone())
                .unwrap_or_default()
        };
        if !restored.is_empty() {
            tracing::info!(
                "[RUNNER {id}] KV restored from persisted state: {} keys",
                restored.len()
            );
            append_runner_log(&store, &id, &format!(
                "KV restored: {} state keys (avg_vol={:.4}, loss_streak={:.0})",
                restored.len(),
                restored.get("avg_vol").copied().unwrap_or(0.0),
                restored.get("loss_streak").copied().unwrap_or(0.0),
            ));
            restored
        } else {
            let init_decision_minute = (window_minutes as i64) - 1;
            let res_logic = config.resolution_logic.as_deref().unwrap_or("price_up");
            match crate::tools::backtest::run_polymarket_bt_signal_preview(
                &script_content,
                buffer.iter().cloned().collect(),
                window_minutes,
                Some(init_decision_minute),
                res_logic,
                config.threshold,
                config.initial_balance,
                config.price_mode.as_deref().unwrap_or("historical"),
                0.0,
                0.0, // warmup: no real price, use momentum model
                0.0, // warmup: no prev P3, drift = 0 (script must handle gracefully)
            ) {
                Ok(bt_seed) => {
                    let state: std::collections::HashMap<String, f64> = bt_seed.kv_state.iter()
                        .filter(|(k, _)| !k.starts_with("debug_"))
                        .map(|(k, v)| (k.clone(), *v))
                        .collect();
                    tracing::info!("[RUNNER {id}] KV pre-seeded from BT warmup: {} state keys", state.len());
                    append_runner_log(&store, &id, &format!(
                        "KV warmup: avg_vol={:.4}",
                        state.get("avg_vol").copied().unwrap_or(0.0)
                    ));
                    state
                }
                Err(e) => {
                    tracing::warn!("[RUNNER {id}] BT warmup seed failed ({}), starting with empty kv", e);
                    std::collections::HashMap::new()
                }
            }
        }
    };
    // Live-mode counters. Restored from the previously persisted result so
    // KPIs survive a pause/restart cycle. If no prior result exists (fresh
    // runner), these start at zero.
    let (mut live_orders, mut live_wins, mut live_total_trades): (Vec<LiveOrder>, u32, u32) = {
        let map = store.runners.lock().unwrap();
        if let Some(r) = map.get(&id) {
            if let Some(ref prev) = r.result {
                (prev.live_orders.clone(), prev.live_wins, prev.live_total_trades)
            } else {
                (Vec::new(), 0, 0)
            }
        } else {
            (Vec::new(), 0, 0)
        }
    };
    if !live_orders.is_empty() {
        tracing::info!(
            "[RUNNER {id}] Restored state on start: {} orders, {} wins, {} total trades",
            live_orders.len(), live_wins, live_total_trades
        );
        append_runner_log(
            &store, &id,
            &format!(
                "Restored on restart: {} trades ({} wins, {:.1}% WR)",
                live_total_trades, live_wins,
                if live_total_trades > 0 { (live_wins as f64 / live_total_trades as f64) * 100.0 } else { 0.0 }
            ),
        );
    }

    // Minute within the window to take the decision (0-based open_time index).
    // For a 5m window [T … T+300s]:
    //   decision candle  = index 3, open=T+180s, close=T+240s  (fires at T+240s+~1s via WS)
    //   This gives 60 seconds for the order to execute before window resolution at T+300s.
    //   Note: backtest uses index 4 (last candle) since it doesn't need execution time.
    let decision_minute = (window_minutes as i64) - 2;

    // early_fire_secs: fire order N seconds before the decision candle closes.
    // Resolved at runner creation (api.rs merges per-runner override with global config.toml).
    let early_fire_secs = config.early_fire_secs.unwrap_or(0) as i64;

    // Pinned sleep used for the optional early-fire timer.
    // Initialized to far future so it never fires until armed.
    let far_future = tokio::time::Instant::now() + std::time::Duration::from_secs(86400 * 365);
    let early_sleep = tokio::time::sleep_until(far_future);
    tokio::pin!(early_sleep);
    let mut early_fire_armed_window: i64 = -1; // window_ts that the timer is armed for

    // ── Watchdog ──────────────────────────────────────────────────────────
    // Defensive: if no candle arrives for >WATCHDOG_STALL_SECS, the WS feed
    // is stuck (network partition, Binance restart, etc.). We break out of
    // the runner loop so the caller (spawn site) can decide whether to
    // auto-restart. Binance normally sends a candle every minute; >5 min
    // silence is already unusual, >10 min is almost certainly a stuck
    // connection not recovering on its own.
    const WATCHDOG_STALL_SECS: u64 = 600;        // 10 min → break & exit runner
    const WATCHDOG_WARN_SECS: u64 = 300;         // 5 min → log warning
    const WATCHDOG_POLL_SECS: u64 = 60;          // how often to check
    let mut last_candle_at = tokio::time::Instant::now();
    let mut watchdog_warned = false;
    let watchdog_sleep = tokio::time::sleep(std::time::Duration::from_secs(WATCHDOG_POLL_SECS));
    tokio::pin!(watchdog_sleep);

    // Always false now: the watchdog reconnects the feed in-place rather than
    // exiting the runner. Kept so the post-loop block stays a no-op without
    // needing to remove that branch.
    let watchdog_tripped = false;
    loop {
        // Either a new closed candle arrives, the early-fire timer fires,
        // or the watchdog wakes up to check feed health.
        let early_fired = tokio::select! {
            candle_opt = candle_rx.recv() => {
                let live_inner = match candle_opt { Some(c) => c, None => break };
                // Shadow `live` for the rest of the block
                let live = live_inner;
                // Reset watchdog — a candle arrived so the feed is healthy.
                last_candle_at = tokio::time::Instant::now();
                watchdog_warned = false;

                let candle = crate::tools::backtest::Candle {
                    open_time_ms: live.open_time_ms,
                    open: live.open, high: live.high, low: live.low,
                    close: live.close, volume: live.volume,
                };
                buffer.push_back(candle.clone());
                if buffer.len() > MAX_BUFFER { buffer.pop_front(); }

                // ── Arm early-fire timer when the candle BEFORE the decision arrives ──
                // e.g. for 5m window with early_fire_secs=10: minute 2 candle arrives
                // at ~T+2:00; we want to fire at T+3:50 (10s before minute-3 close).
                if early_fire_secs > 0 && live.open_time_ms != 0 {
                    let candle_ts = live.open_time_ms / 1000;
                    let win = candle_ts - (candle_ts % window_secs as i64);
                    let min_in_win = (candle_ts % window_secs as i64) / 60;
                    if min_in_win == decision_minute - 1 && win != early_fire_armed_window && win != last_decision_window {
                        // Decision candle closes at decision_minute * 60 into the window.
                        let decision_close_ts = win + decision_minute * 60 + 60; // +60 = end of decision candle
                        let fire_ts = decision_close_ts - early_fire_secs;
                        let now_ts = chrono::Utc::now().timestamp();
                        let delay_ms = ((fire_ts - now_ts).max(0) as u64) * 1000;
                        early_sleep.as_mut().reset(
                            tokio::time::Instant::now() + std::time::Duration::from_millis(delay_ms)
                        );
                        early_fire_armed_window = win;
                        tracing::info!("[RUNNER {id}] Early fire armed for window {} — fires in {:.1}s", win, delay_ms as f64 / 1000.0);
                        append_runner_log(&store, &id, &format!("Early fire armed: {}s before candle close", early_fire_secs));
                    }
                }

                // ── Drift signal: capture YES token price 60s before the decision ──
                // Fires exactly once per window when the candle preceding the decision
                // arrives (min_in_win == decision_minute - 1). Result is consumed at
                // the decision point as `ctx.token_price_prev` for `ctx.token_drift`.
                if live.open_time_ms != 0 {
                    let candle_ts = live.open_time_ms / 1000;
                    let win = candle_ts - (candle_ts % window_secs as i64);
                    let min_in_win = (candle_ts % window_secs as i64) / 60;
                    if min_in_win == decision_minute - 1
                        && win != prev_yes_captured_window
                        && win != last_decision_window
                    {
                        if let (Some(yes), Some(no)) = (&config.poly_token_id, &config.poly_no_token_id) {
                            let (p3_yes, p3_no) = fetch_token_prices(yes, no).await;
                            if p3_yes > 0.0 && p3_yes < 1.0 {
                                prev_yes_token_price = p3_yes;
                                prev_yes_captured_window = win;
                                tracing::info!(
                                    "[RUNNER {id}] P3 captured for window {}: yes={:.4} no={:.4} sum={:.4}",
                                    win, p3_yes, p3_no, p3_yes + p3_no
                                );
                                append_runner_log(
                                    &store, &id,
                                    &format!("P3 captured: yes={:.4} no={:.4} sum={:.4}", p3_yes, p3_no, p3_yes + p3_no)
                                );
                            } else {
                                tracing::warn!("[RUNNER {id}] P3 fetch returned invalid price ({}); drift signal unavailable", p3_yes);
                            }
                        }
                    }
                }

                false // not an early fire event
            }
            _ = &mut early_sleep, if early_fire_armed_window != -1 && early_fire_armed_window != last_decision_window => {
                // Reset timer to far future so it doesn't keep firing
                early_sleep.as_mut().reset(tokio::time::Instant::now() + std::time::Duration::from_secs(86400 * 365));
                tracing::info!("[RUNNER {id}] Early fire triggered for window {}", early_fire_armed_window);
                append_runner_log(&store, &id, "Early fire: placing order before candle close");
                true // signal that this is an early-fire tick
            }
            _ = &mut watchdog_sleep => {
                // Wake up every WATCHDOG_POLL_SECS to check feed health.
                // Re-arm the timer so we poll again.
                watchdog_sleep.as_mut().reset(
                    tokio::time::Instant::now() + std::time::Duration::from_secs(WATCHDOG_POLL_SECS)
                );
                let stale_secs = last_candle_at.elapsed().as_secs();
                if stale_secs >= WATCHDOG_STALL_SECS {
                    // Feed is zombie — drop the receiver (which kills the
                    // background WS task) and spawn a fresh one. Keeps the
                    // runner alive across OS sleep / lid-close / network blip.
                    tracing::warn!(
                        "[RUNNER {id}] WATCHDOG: no candle for {}s (>{}s limit) — respawning feed",
                        stale_secs, WATCHDOG_STALL_SECS
                    );
                    append_runner_log(
                        &store, &id,
                        &format!(
                            "WATCHDOG: no WS candle for {}s — reconnecting feed (runner stays alive)",
                            stale_secs
                        ),
                    );
                    candle_rx = crate::live_feed::spawn_binance_kline_feed(
                        binance_sym.clone(), "1m".to_string(),
                    );
                    last_candle_at = tokio::time::Instant::now();
                    watchdog_warned = false;
                    continue;
                } else if stale_secs >= WATCHDOG_WARN_SECS && !watchdog_warned {
                    tracing::warn!(
                        "[RUNNER {id}] WATCHDOG: no candle for {}s (warn threshold {}s)",
                        stale_secs, WATCHDOG_WARN_SECS
                    );
                    append_runner_log(
                        &store, &id,
                        &format!("WATCHDOG warning: no WS candle for {}s. Will stop at {}s.", stale_secs, WATCHDOG_STALL_SECS),
                    );
                    watchdog_warned = true;
                }
                // Not an early-fire event. Skip the rest of the iteration.
                continue;
            }
        };

        // When early fire triggers, synthesize window/candle timing from wall-clock.
        let (live, current_window, next_window, minute_in_window) = if early_fired {
            let now_ts = chrono::Utc::now().timestamp();
            let win = now_ts - (now_ts % window_secs as i64);
            let next_win = win + window_secs as i64;
            let min_in_win = (now_ts % window_secs as i64) / 60;
            // Use the last candle in the buffer as the "live" candle for context
            let synth_live = buffer.back().map(|c| crate::live_feed::LiveCandle {
                open_time_ms: c.open_time_ms,
                open: c.open, high: c.high, low: c.low, close: c.close, volume: c.volume,
            }).unwrap_or_else(|| crate::live_feed::LiveCandle {
                open_time_ms: now_ts * 1000, open: 0.0, high: 0.0, low: 0.0, close: 0.0, volume: 0.0,
            });
            (synth_live, win, next_win, min_in_win)
        } else {
            // Normal candle path: recompute from buffer's last candle (just pushed)
            let last = buffer.back().unwrap();
            let candle_ts_secs = last.open_time_ms / 1000;
            let win = candle_ts_secs - (candle_ts_secs % window_secs as i64);
            let next_win = win + window_secs as i64;
            let min_in_win = (candle_ts_secs % window_secs as i64) / 60;
            let synth_live = crate::live_feed::LiveCandle {
                open_time_ms: last.open_time_ms,
                open: last.open, high: last.high, low: last.low, close: last.close, volume: last.volume,
            };
            (synth_live, win, next_win, min_in_win)
        };

        // Track price history for live chart (keep last 60 points)
        if !early_fired {
            price_history.push_back((live.open_time_ms, live.close));
            if price_history.len() > 60 { price_history.pop_front(); }
        }

        let window_seconds_left = next_window - (live.open_time_ms / 1000);

        // ── New window boundary: resolve previous window, prepare tokens ──
        //
        // IMPORTANT: do NOT advance `last_window` until we have successfully
        // resolved the tokens for this window. If we advance eagerly and the
        // resolve fails (network blip), we would silently bet against stale
        // tokens from the previous window — which is a near-resolution market
        // with extreme prices and thin books.
        if current_window != last_window {
            tracing::info!(
                "[RUNNER {id}] New {}-min window @ {} UTC",
                window_minutes,
                chrono::DateTime::from_timestamp(current_window, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_default()
            );

            // Resolve tokens for the new window. `resolve_token_for_window`
            // internally retries with backoff (3 attempts over ~10s).
            let resolve_ok = if let Some(ref series_id) = config.series_id {
                match resolve_token_for_window(series_id, current_window as u64).await {
                    Ok((yes_id, no_id, condition_id)) => {
                        tracing::info!(
                            "[RUNNER {id}] Resolved tokens for window {}: YES={} NO={} condition_id={}",
                            current_window, yes_id, no_id, condition_id
                        );
                        // Update BOTH the store and the local config so that
                        // order placement and live-feed price checks use the
                        // current window's tokens, not stale ones.
                        config.poly_token_id = Some(yes_id.clone());
                        config.poly_no_token_id = Some(no_id.clone());
                        config.poly_condition_id = Some(condition_id.clone());
                        let mut map = store.runners.lock().unwrap();
                        if let Some(r) = map.get_mut(&id) {
                            r.config.poly_token_id = Some(yes_id);
                            r.config.poly_no_token_id = Some(no_id);
                            r.config.poly_condition_id = Some(condition_id.clone());
                        }
                        drop(map);
                        // Fetch and cache historical trades for this market window
                        let ws = workspace_dir.clone();
                        let cond = condition_id.clone();
                        tokio::spawn(async move {
                            update_trade_cache(&cond, &ws).await;
                        });
                        true
                    }
                    Err(e) => {
                        tracing::error!(
                            "[RUNNER {id}] Failed to resolve token_id for window {} after retries: {}",
                            current_window, e
                        );
                        append_runner_log(
                            &store, &id,
                            &format!(
                                "Token resolution failed for window {} after 3 retries: {} — clearing stale tokens, will retry on next candle",
                                current_window, e
                            ),
                        );
                        // Clear stale tokens so subsequent price fetches don't
                        // accidentally read from the previous window's market.
                        config.poly_token_id = None;
                        config.poly_no_token_id = None;
                        config.poly_condition_id = None;
                        let mut map = store.runners.lock().unwrap();
                        if let Some(r) = map.get_mut(&id) {
                            r.config.poly_token_id = None;
                            r.config.poly_no_token_id = None;
                            r.config.poly_condition_id = None;
                        }
                        false
                    }
                }
            } else {
                // No series configured = manual market selection; consider "ok"
                true
            };

            if resolve_ok {
                last_window = current_window;
            } else {
                // If resolve failed, `last_window` stays at its previous value
                // so the very next candle (~60s) will retry the whole block.
                // Skip the rest of the window-transition work (backtest eval,
                // previous-window resolution, tick update, balance fetch) because
                // it all depends on valid tokens. We still proceed to the normal
                // candle processing below.
                //
                // Advance the `last_tick_at` heartbeat anyway so the dashboard
                // knows the runner is alive, just stalled on token resolution.
                let now = chrono::Utc::now().to_rfc3339();
                let mut map = store.runners.lock().unwrap();
                if let Some(r) = map.get_mut(&id) {
                    r.status.last_tick_at = Some(now);
                }
                continue;
            }

            // Run backtest on the full buffer so we can compare backtest vs live
            // for the window that just completed.
            let metrics = eval_polymarket(&script_content, &buffer, window_minutes, &config);

            // Resolve previous window outcome and compare with backtest
            if let Some((prev_window, prev_signal, prev_bt_signal, prev_debug)) = prev_live_position.take() {
                    let res_logic = config.resolution_logic.as_deref().unwrap_or("price_up");

                    // Prefer Polymarket's own resolution (matches what the chain paid);
                    // fall back to Binance candle inference only when the market
                    // hasn't been reconciled by Polymarket yet.
                    let (resolved_via_poly, went_up_opt, resolution_provisional) =
                        match config.series_id.as_deref() {
                        Some(sid) if !sid.is_empty() => {
                            // condition_id stored on the prev-window order → robust CLOB lookup
                            let prev_cid: Option<String> = {
                                let map = store.runners.lock().unwrap();
                                map.get(&id)
                                    .and_then(|r| r.result.as_ref())
                                    .and_then(|res| res.live_orders.iter().find(|o| o.window_ts == prev_window))
                                    .and_then(|o| o.condition_id.clone())
                            };
                            match fetch_polymarket_resolution(sid, prev_window, prev_cid.as_deref()).await {
                                Some(yes_won) => (true, Some(yes_won), false),
                                None => {
                                    // Not settled yet — use Binance as provisional,
                                    // spawn monitor to upgrade when oracle arrives
                                    let binance_res = resolve_window_outcome(
                                        &buffer, prev_window, window_secs as i64, res_logic, config.threshold,
                                    );
                                    (false, binance_res, true)
                                }
                            }
                        }
                        _ => (false, resolve_window_outcome(
                            &buffer, prev_window, window_secs as i64, res_logic, config.threshold,
                        ), false),
                    };
                    let resolution_source_str = if resolved_via_poly {
                        "polymarket"
                    } else if resolution_provisional {
                        "binance_provisional"
                    } else {
                        "binance"
                    };

                    // Spawn resolution monitor for provisional results.
                    // on_candle engine rebuilds balance from sum(order.pnl) each cycle,
                    // so the monitor only needs to patch order.pnl (tick_state = None).
                    if resolution_provisional {
                        if let Some(ref sid) = config.series_id {
                            if let Some(binance_guess) = went_up_opt {
                                spawn_resolution_monitor(
                                    id.clone(),
                                    sid.clone(),
                                    prev_window,
                                    store.clone(),
                                    binance_guess,
                                    None, // on_candle: balance recalculated from orders
                                    config.fee_pct,
                                );
                            }
                        }
                    }
                    if let Some(went_up) = went_up_opt {
                        let outcome = if went_up { "UP" } else { "DOWN" };

                        // ── Backtest vs Live comparison for this window ──
                        let decision_ts = prev_window + ((window_minutes as i64) - 1) * 60;
                        let decision_dt = chrono::DateTime::from_timestamp(decision_ts, 0)
                            .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
                            .unwrap_or_default();

                        // Use the BT preview signal captured at decision time for comparison.
                        // The full backtest (all_trades) can show "flat" simply because the
                        // simulated balance was depleted — that's a capital constraint, not a
                        // signal divergence.  The preview signal is stateless and reflects
                        // what the strategy logic would say given the same candle data.
                        let bt_signal = prev_bt_signal.clone();

                        // Still pull debug and pnl from all_trades when available.
                        let bt_trade = metrics.all_trades.iter()
                            .find(|t| t.timestamp == decision_dt);
                        let bt_debug: Option<std::collections::HashMap<String, f64>> =
                            bt_trade.and_then(|t| t.debug.clone())
                            .or_else(|| metrics.flat_debugs.iter()
                                .find(|(ts, _)| *ts == decision_dt)
                                .map(|(_, d)| d.clone()));

                        // Signal direction: ignore sizing suffix (e.g. "yes 10" → "yes")
                        let live_dir = prev_signal.split_whitespace().next().unwrap_or("flat");
                        let bt_dir   = bt_signal.split_whitespace().next().unwrap_or("flat");

                        if live_dir != bt_dir {
                            let bt_pnl_note = bt_trade
                                .map(|t| format!(" bt_pnl={:.2}", t.pnl))
                                .unwrap_or_default();
                            append_runner_log(
                                &store, &id,
                                &format!(
                                    "DISCREPANCY window {}: live={} but backtest={}{}",
                                    prev_window, live_dir, bt_dir, bt_pnl_note
                                ),
                            );
                        }

                        // Only count trades and log win/loss when live actually placed a bet.
                        if prev_signal.starts_with("flat") {
                            tracing::info!(
                                "[RUNNER {id}] Window {} resolved {}. Position FLAT",
                                prev_window, outcome
                            );
                            append_runner_log(
                                &store, &id,
                                &format!("Window {}: {} | Position FLAT", prev_window, outcome),
                            );
                        } else {
                            // Only count this as a trade if a real fill exists for this window.
                            // On-chain orders count when status == "MATCHED" (CLOB confirmed
                            // fill). Paper orders are simulated as filled at decision-time
                            // entry_price, so they always count regardless of status.
                            let has_order = live_orders.iter().any(|o| {
                                o.window_ts == prev_window
                                    && (o.status == "MATCHED" || o.order_id.starts_with("paper-"))
                            });
                            if !has_order {
                                tracing::info!(
                                    "[RUNNER {id}] Window {} resolved {}. Signal={} but NO ORDER PLACED",
                                    prev_window, outcome, prev_signal
                                );
                                append_runner_log(
                                    &store, &id,
                                    &format!("Window {}: {} | Signal {} | NO ORDER PLACED", prev_window, outcome, prev_signal),
                                );
                            } else {
                                let won = (prev_signal.starts_with("yes") && went_up)
                                    || (prev_signal.starts_with("no") && !went_up);
                                live_total_trades += 1;
                                if won { live_wins += 1; }
                                let pos = if prev_signal.starts_with("yes") { "YES" } else { "NO" };
                                let result = if won { "WIN" } else { "LOSS" };
                                // Update matching order with result and P&L
                                for order in live_orders.iter_mut() {
                                    if order.window_ts == prev_window && !order.stop_loss_triggered {
                                        // Stamp the resolution we just computed so the
                                        // backfill / dashboard can audit the source.
                                        order.resolution_yes_won = Some(went_up);
                                        order.resolution_source = Some(resolution_source_str.to_string());

                                        // Skip LIVE (unfilled) on-chain orders — they never
                                        // received tokens. Paper orders (id prefix "paper-")
                                        // are simulated and always settle at entry_price.
                                        let is_paper = order.order_id.starts_with("paper-");
                                        if !is_paper && order.status != "MATCHED" {
                                            tracing::warn!(
                                                "[RUNNER {id}] Window {} settle: order status='{}' (not MATCHED); skipping P&L",
                                                prev_window, order.status
                                            );
                                            order.result = Some("UNFILLED".to_string());
                                            order.pnl = Some(0.0);
                                            if won { live_wins = live_wins.saturating_sub(1); }
                                            live_total_trades = live_total_trades.saturating_sub(1);
                                            continue;
                                        }
                                        // Prefer the **real on-chain fill price** when we got
                                        // it back from CLOB /data/trades. Fall back to the
                                        // decision-time midpoint (capped to 0.10) otherwise —
                                        // those orders carry an asterisk so we can spot them.
                                        //
                                        // Guard against a poisoned fill_price: a simulated paper
                                        // fill on the illiquid NON-favourite book can land at ~0.99
                                        // while the decision ask (entry_price) was ~0.44. A market
                                        // buy never fills more than ~25% above the ask, so reject a
                                        // fill that diverges that far and use entry_price instead.
                                        let decision_ep = order.entry_price.unwrap_or(0.5);
                                        let fill_is_sane = |fp: f64| {
                                            fp >= 0.01 && fp <= 0.99
                                                && (decision_ep <= 0.0 || fp <= decision_ep * 1.25)
                                        };
                                        let (ep, suspect) = match order.fill_price {
                                            Some(fp) if fill_is_sane(fp) => (fp, false),
                                            Some(fp) => {
                                                tracing::warn!(
                                                    "[RUNNER {id}] Window {} settle: fill_price {:.4} diverges from decision ask {:.4} (>25%); using entry_price",
                                                    prev_window, fp, decision_ep
                                                );
                                                if decision_ep < 0.10 { (0.50, true) } else { (decision_ep.max(0.10), true) }
                                            }
                                            None => {
                                                let raw_ep = decision_ep;
                                                if raw_ep < 0.10 {
                                                    tracing::warn!(
                                                        "[RUNNER {id}] Window {} settle: entry_price {:.4} below trust floor 0.10 and no fill_price; capping to 0.50",
                                                        prev_window, raw_ep
                                                    );
                                                    (0.50, true)
                                                } else {
                                                    (raw_ep.max(0.10), true)
                                                }
                                            }
                                        };
                                        // Apply fee_pct to the winning payout for parity with
                                        // the backtest on_candle model (net_pay = tokens × (1−fee%)).
                                        let pnl = if won {
                                            let gross_payout = order.amount_usdc / ep;
                                            let net_payout = gross_payout * (1.0 - config.fee_pct / 100.0);
                                            net_payout - order.amount_usdc
                                        } else {
                                            -order.amount_usdc
                                        };
                                        order.result = Some(if suspect {
                                            format!("{}*", result)
                                        } else {
                                            result.to_string()
                                        });
                                        order.pnl = Some(pnl);
                                    }
                                }
                                tracing::info!(
                                    "[RUNNER {id}] Window {} resolved {}. Position {} → {} (live win rate: {:.1}%)",
                                    prev_window, outcome, pos, result,
                                    if live_total_trades > 0 { live_wins as f64 / live_total_trades as f64 * 100.0 } else { 0.0 }
                                );
                                append_runner_log(
                                    &store, &id,
                                    &format!("Window {}: {} | Position {} → {}", prev_window, outcome, pos, result),
                                );

                                // ── Auto-record to dynamic asset selector ──────────────────
                                // Compute rough P&L for the asset selector (use order if available)
                                let sel_pnl = live_orders.iter()
                                    .find(|o| o.window_ts == prev_window && !o.stop_loss_triggered)
                                    .and_then(|o| o.pnl)
                                    .unwrap_or(if won { 1.0 } else { -1.0 });
                                let script_name = config.script
                                    .rsplit('/').next().unwrap_or(&config.script).to_string();
                                let ws_for_sel = workspace_dir.clone();
                                let sym_for_sel = config.symbol.clone();
                                tokio::spawn(async move {
                                    crate::tools::asset_selector::record_trade(
                                        &ws_for_sel, &script_name, &sym_for_sel,
                                        prev_window, won, sel_pnl,
                                    ).await;
                                });
                            }
                        }

                        // Always print debug indicators for both live and backtest
                        let live_debug_str = format_debug_values(&prev_debug);
                        let bt_debug_map = bt_debug.unwrap_or_default();
                        let bt_debug_str = format_debug_values(&bt_debug_map);
                        append_runner_log(
                            &store, &id,
                            &format!(
                                "INDICATORS window {}: LIVE[signal={} {}] | BT[signal={} {}]",
                                prev_window,
                                prev_signal,
                                if live_debug_str.is_empty() { "(no debug)".to_string() } else { live_debug_str },
                                bt_signal,
                                if bt_debug_str.is_empty() { "(no debug)".to_string() } else { bt_debug_str }
                            ),
                        );
                    }
                }

            let now = chrono::Utc::now().to_rfc3339();
            let next_ts = chrono::DateTime::from_timestamp(next_window, 0)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default();
            {
                let mut map = store.runners.lock().unwrap();
                if let Some(r) = map.get_mut(&id) {
                    r.status.last_tick_at = Some(now);
                    r.status.next_tick_at = Some(next_ts);
                }
            }
            let wallet_balance = if let Some(ref client) = clob_client {
                // Live mode: read real USDC balance from the CLOB/exchange
                fetch_usdc_balance_clob(client).await
            } else {
                // Adopt any official resolution the sweep / monitor wrote to the store
                // since this loop last touched `live_orders`. Without this, the
                // `live_orders.clone()` write-back below clobbers those upgrades back to
                // binance_provisional (the loop owns a local Vec it overwrites each cycle).
                {
                    let map = store.runners.lock().unwrap();
                    if let Some(stored) = map.get(&id).and_then(|r| r.result.as_ref()) {
                        for lo in live_orders.iter_mut() {
                            if lo.resolution_source.as_deref() == Some("polymarket") { continue; }
                            if let Some(so) = stored.live_orders.iter().find(|s| {
                                s.window_ts == lo.window_ts && s.order_id == lo.order_id
                            }) {
                                if so.resolution_source.as_deref() == Some("polymarket") {
                                    lo.resolution_source = so.resolution_source.clone();
                                    lo.resolution_yes_won = so.resolution_yes_won;
                                    lo.result = so.result.clone();
                                    lo.pnl = so.pnl;
                                }
                            }
                        }
                    }
                }
                // Recompute the win counter from the (now synced) results so the
                // dashboard win-rate reflects official outcomes after any flip.
                live_wins = live_orders.iter()
                    .filter(|o| o.result.as_deref().map(|r| r.starts_with("WIN")).unwrap_or(false))
                    .count() as u32;

                // Paper mode: running balance = initial + sum of all settled pnl.
                let settled_pnl: f64 = live_orders.iter().filter_map(|o| o.pnl).sum();
                Some(config.initial_balance + settled_pnl)
            };
            update_runner_result(
                &store, &id, &config, &metrics, None,
                config.wallet_address.clone(),
                wallet_balance,
                Some(live_orders.clone()),
                Some(live_wins),
                Some(live_total_trades),
                None,
            ).await;
            store.persist();
        }

        // ── Decision point: evaluate strategy and place order (once per window) ──
        if minute_in_window == decision_minute && current_window != last_decision_window {
            last_decision_window = current_window;

            // ── Guardrail: max_runner_loss_pct ──────────────────────────────────────
            if let Some(max_loss_pct) = config.max_runner_loss_pct {
                if max_loss_pct > 0.0 {
                    let settled_pnl: f64 = live_orders.iter().filter_map(|o| o.pnl).sum();
                    let loss_pct = -settled_pnl / config.initial_balance.max(1.0);
                    if loss_pct >= max_loss_pct {
                        let msg = format!(
                            "AUTO-STOP: max_runner_loss_pct {:.0}% breached (down {:.1}% = ${:.2}). Switching to paper.",
                            max_loss_pct * 100.0, loss_pct * 100.0, -settled_pnl
                        );
                        tracing::warn!("[RUNNER {id}] {msg}");
                        append_runner_log(&store, &id, &msg);
                        // Switch to paper mode in the config and store so it doesn't restart live
                        {
                            let mut map = store.runners.lock().unwrap();
                            if let Some(r) = map.get_mut(&id) {
                                r.config.mode = "paper".to_string();
                            }
                        }
                        config.mode = "paper".to_string();
                        clob_client = None; // Switch to paper mode in-loop
                        store.persist();
                    }
                }
            }

            // ── Guardrail: max_consecutive_losses ───────────────────────────────────
            let consecutive_losses: u32 = {
                let settled: Vec<_> = live_orders.iter()
                    .filter(|o| o.pnl.is_some())
                    .collect();
                let mut streak = 0u32;
                for o in settled.iter().rev() {
                    if (o.pnl.unwrap_or(0.0)) < 0.0 {
                        streak += 1;
                    } else {
                        break;
                    }
                }
                streak
            };
            if let Some(max_streak) = config.max_consecutive_losses {
                if max_streak > 0 && consecutive_losses >= max_streak {
                    let msg = format!(
                        "AUTO-STOP: {} consecutive losses (max={}). Switching to paper.",
                        consecutive_losses, max_streak
                    );
                    tracing::warn!("[RUNNER {id}] {msg}");
                    append_runner_log(&store, &id, &msg);
                    {
                        let mut map = store.runners.lock().unwrap();
                        if let Some(r) = map.get_mut(&id) {
                            r.config.mode = "paper".to_string();
                        }
                    }
                    config.mode = "paper".to_string();
                    clob_client = None;
                    store.persist();
                }
            }

            let display_minute = decision_minute + 1; // 1-based for user clarity
            tracing::info!(
                "[RUNNER {id}] Decision point for window {} (minute {}/{}, candle close at {:02}:{:02}), evaluating strategy...",
                current_window, display_minute, window_minutes,
                ((current_window % 86400) / 3600) as u64,
                ((current_window % 3600) / 60) as u64
            );

            // Log the candles that the strategy will see for this window
            let window_candles: Vec<String> = buffer
                .iter()
                .filter(|c| {
                    let ts = c.open_time_ms / 1000;
                    ts >= current_window && ts < current_window + window_secs as i64
                })
                .map(|c| {
                    let ts = c.open_time_ms / 1000;
                    format!(
                        "{} {:02}:{:02} O={:.2} H={:.2} L={:.2} C={:.2} V={:.5}",
                        ts,
                        ((ts % 86400) / 3600) as u64,
                        ((ts % 3600) / 60) as u64,
                        c.open,
                        c.high,
                        c.low,
                        c.close,
                        c.volume
                    )
                })
                .collect();
            if !window_candles.is_empty() {
                let candles_log = window_candles.join(" | ");
                tracing::info!("[RUNNER {id}] Window candles: {}", candles_log);
                append_runner_log(&store, &id, &format!("Candles: {}", candles_log));
            }

            // Fetch real token prices so the strategy sees actual market pricing
            let (yes_token_price, no_token_price) =
                match (&config.poly_token_id, &config.poly_no_token_id) {
                    (Some(yes), Some(no)) => fetch_token_prices(yes, no).await,
                    _ => (0.0, 0.0),
                };
            // Diagnostic: P4 captured at decision. yes+no should ≈ 1.0. Surfaces
            // spread/liquidity anomalies side-by-side with the captured P3 log.
            if yes_token_price > 0.0 && no_token_price > 0.0 {
                append_runner_log(
                    &store, &id,
                    &format!(
                        "P4 captured: yes={:.4} no={:.4} sum={:.4}",
                        yes_token_price, no_token_price, yes_token_price + no_token_price
                    ),
                );
            }

            // ── Spread gate ──
            // When yes_mid + no_mid deviates materially from 1.0, the book is
            // wide enough that paper fills at mid are optimistic vs. realistic
            // execution. BT implicitly assumes no = 1 - yes, so these windows
            // are semantically outside the backtest distribution. Force flat
            // here instead of acting on an artificially favorable entry price.
            // Default 3% ≈ 6¢ combined spread; set max_spread_pct to a large
            // value (e.g. 1.0) in the runner config to disable.
            let spread_pct = if yes_token_price > 0.0 && no_token_price > 0.0 {
                (yes_token_price + no_token_price - 1.0).abs()
            } else {
                0.0
            };
            let spread_guard_threshold = config.max_spread_pct.unwrap_or(0.03);
            let spread_guard_tripped = yes_token_price > 0.0
                && no_token_price > 0.0
                && spread_pct > spread_guard_threshold;
            if spread_guard_tripped {
                tracing::warn!(
                    "[RUNNER {id}] Spread gate tripped for window {}: spread={:.4} > max={:.4}; forcing flat",
                    current_window, spread_pct, spread_guard_threshold
                );
                append_runner_log(
                    &store, &id,
                    &format!(
                        "Spread gate TRIPPED: forcing flat (spread={:.2}% > max={:.2}%)",
                        spread_pct * 100.0, spread_guard_threshold * 100.0
                    ),
                );
            }

            // ── Allowed-hours gate ──
            // Skip decision windows in UTC hours that historically show poor WR.
            // Only active when `allowed_hours` is non-empty.
            let hour_gate_tripped = if !config.allowed_hours.is_empty() {
                let window_hour = (current_window % 86400) / 3600;
                let window_hour_u8 = window_hour as u8;
                !config.allowed_hours.contains(&window_hour_u8)
            } else {
                false
            };
            if hour_gate_tripped {
                let window_hour = (current_window % 86400) / 3600;
                append_runner_log(
                    &store, &id,
                    &format!("Hour gate: skipping window (UTC {:02}:00 not in allowed_hours)", window_hour),
                );
            }

            // ── RV floor gate ──
            // Skip when BTC realized-vol is below the minimum threshold.
            // Flat-market conditions degrade the drift signal to noise.
            let rv_gate_tripped = if let Some(rv_min) = config.rv_min_btc {
                if rv_min > 0.0 {
                    let rv_now = compute_btc_rv_1h(&buffer);
                    let rv_str = rv_now.map(|v| format!("{:.6}", v)).unwrap_or_else(|| "n/a".to_string());
                    let tripped = rv_now.map(|v| v < rv_min).unwrap_or(false);
                    if tripped {
                        append_runner_log(
                            &store, &id,
                            &format!("RV gate: skipping window (rv_btc={} < min={})", rv_str, rv_min),
                        );
                    }
                    tripped
                } else {
                    false
                }
            } else {
                false
            };

            // Live signal: run script on the CURRENT (incomplete) window's decision candle.
            // This is NOT a backtest — it extracts buy/sell intent for the live market.
            let price_mode = config.price_mode.as_deref().unwrap_or("historical");
            // Use captured P3 only if it was fetched for THIS window (avoids stale
            // value from a prior window leaking in if the prior decision was missed).
            let drift_prev = if prev_yes_captured_window == current_window {
                prev_yes_token_price
            } else {
                0.0
            };
            // ── Build oracle comparison data for ctx injection ────────────────
            let binance_now = *latest_binance_price.read().unwrap();
            let (chainlink_now, oracle_lag_secs) = if let Some(ref cl) = chainlink_price {
                let cl_val = *cl.read().await;
                let lag = if cl_val.is_some() && binance_now > 0.0 {
                    // Estimate lag as the configured poll interval (worst-case staleness)
                    config.chainlink_interval_secs as f64
                } else {
                    0.0
                };
                (cl_val.unwrap_or(0.0), lag)
            } else {
                (0.0, 0.0)
            };
            let oracle_data = if binance_now > 0.0 {
                Some((binance_now, chainlink_now, oracle_lag_secs))
            } else {
                None
            };
            // minute_offset: 0 = standard decision candle; >0 = early fire (minutes before window close)
            let cur_signal_minute_offset: i64 = if early_fired {
                // Fired early: offset = how many minutes before the last candle
                (window_minutes as i64) - 1 - decision_minute
            } else {
                0
            };
            if binance_now > 0.0 || chainlink_now > 0.0 {
                append_runner_log(
                    &store, &id,
                    &format!("Oracle: binance=${:.2} chainlink=${:.2} lag={:.0}s offset={}m",
                        binance_now, chainlink_now, oracle_lag_secs, cur_signal_minute_offset),
                );
            }

            let live_result = match crate::tools::backtest::run_polymarket_live_signal(
                &script_content,
                buffer.iter().cloned().collect(),
                window_minutes,
                Some(decision_minute),
                yes_token_price,
                no_token_price,
                price_mode,
                &live_kv_state,
                drift_prev,
                oracle_data,
                cur_signal_minute_offset,
            ) {
                Ok(res) => res,
                Err(e) => {
                    tracing::warn!("[RUNNER {id}] Live signal eval failed: {}", e);
                    append_runner_log(&store, &id, &format!("Signal eval FAILED: {}", e));
                    crate::tools::backtest::LiveSignalResult {
                        signal: "flat".to_string(),
                        size: 0.0,
                        debug: std::collections::HashMap::new(),
                        kv_state: std::collections::HashMap::new(),
                        binance_mark: 0.0,
                        chainlink_mark: 0.0,
                        oracle_lag_secs: 0.0,
                        minute_offset: 0,
                    }
                }
            };
            // Persist updated kv state for the next window.
            // Only carry real state values (e.g. avg_vol) — strip debug_* keys so that
            // if the script early-returns next window, stale indicator values don't leak.
            live_kv_state = live_result.kv_state.iter()
                .filter(|(k, _)| !k.starts_with("debug_"))
                .map(|(k, v)| (k.clone(), *v))
                .collect();
            // Also mirror it into the store so it persists across pause/restart.
            // Cheap: HashMap<String, f64> with typically <10 keys.
            {
                let mut map = store.runners.lock().unwrap();
                if let Some(r) = map.get_mut(&id) {
                    if let Some(ref mut res) = r.result {
                        res.live_kv_state = live_kv_state.clone();
                    }
                }
            }

            // If any gate tripped, override to "flat". The script's debug values
            // still surface below so we can see what it WOULD have done.
            let current_signal = if spread_guard_tripped || hour_gate_tripped || rv_gate_tripped {
                "flat".to_string()
            } else {
                live_result.signal.clone()
            };
            tracing::info!("[RUNNER {id}] Live signal for window {}: {}", current_window, current_signal);
            append_runner_log(&store, &id, &format!("Signal window {}: {}", current_window, current_signal));

            // Log debug values (indicators) for every window, trade or flat.
            let debug_str = format_debug_values(&live_result.debug);
            if !debug_str.is_empty() {
                append_runner_log(&store, &id, &format!("LIVE debug: {}", debug_str));
            }

            // Run BT-engine preview at the same decision point so the operator
            // can compare BT indicators vs LIVE indicators side-by-side.
            // Capture the signal here; it is stored in prev_live_position for
            // discrepancy detection at the next window tick (avoids false positives
            // when the full BT balance is depleted and all_trades has no entry).
            let res_logic = config.resolution_logic.as_deref().unwrap_or("price_up");
            let bt_preview_signal = match crate::tools::backtest::run_polymarket_bt_signal_preview(
                &script_content,
                buffer.iter().cloned().collect(),
                window_minutes,
                Some(decision_minute),
                res_logic,
                config.threshold,
                config.initial_balance,
                price_mode,
                no_token_price,
                yes_token_price, // real CLOB price so BT preview matches live signal
                drift_prev,
            ) {
                Ok(bt_res) => {
                    append_runner_log(&store, &id, &format!("BT signal: {}", bt_res.signal));
                    let bt_debug_str = format_debug_values(&bt_res.debug);
                    if !bt_debug_str.is_empty() {
                        append_runner_log(&store, &id, &format!("BT debug: {}", bt_debug_str));
                    }
                    bt_res.signal
                }
                Err(e) => {
                    append_runner_log(&store, &id, &format!("BT preview FAILED: {}", e));
                    "flat".to_string()
                }
            };

            // Always record the live decision (flat or trade) so we can compare
            // indicators with backtest when the window resolves.
            prev_live_position = Some((current_window, current_signal.clone(), bt_preview_signal, live_result.debug.clone()));

            if !current_signal.starts_with("flat") {
                // Confidence-weighted sizing: scripts can emit a `kelly_size`
                // value in their kv_state. The runner reads it here and passes
                // as a multiplier (clamped to [0.1, kelly_cap]) on the base stake.
                // Default 1.0 if the script doesn't set it.
                let kelly_cap = config.kelly_size_cap.max(1.0).min(3.0);
                let kelly_mult = live_result.kv_state.get("kelly_size")
                    .copied()
                    .map(|v| v.clamp(0.1, kelly_cap))
                    .unwrap_or(1.0);
                if (kelly_mult - 1.0).abs() > 0.001 {
                    append_runner_log(
                        &store, &id,
                        &format!("Confidence sizing: kelly_size={:.2}x", kelly_mult),
                    );
                }
                // Regressive sizing: reduce bet to 50% after 3+ consecutive losses
                let effective_size = if consecutive_losses >= 3 {
                    let reduced = live_result.size * 0.5;
                    append_runner_log(
                        &store, &id,
                        &format!("Regressive sizing: {} consecutive losses → size {}→{:.4}", consecutive_losses, live_result.size, reduced),
                    );
                    reduced
                } else {
                    live_result.size
                };
                let (order_result, renewed_client) = execute_live_polymarket_signal(
                    &id, clob_client.clone(), &current_signal, effective_size, &config, &live, &store, current_window,
                    next_window,
                    yes_token_price, no_token_price, kelly_mult,
                    config.series_id.as_deref().unwrap_or(""),
                ).await;
                // If credentials were renewed, update the runner's client so all
                // subsequent windows use the fresh L2 session, and persist to disk.
                if let Some(new_client) = renewed_client {
                    if let Some(ref path) = config_path {
                        let creds = new_client.credentials().clone();
                        let path_clone = path.clone();
                        let id_clone = id.clone();
                        tokio::spawn(async move {
                            if let Err(e) = persist_polymarket_creds(&path_clone, &creds).await {
                                tracing::warn!("[RUNNER {id_clone}] Failed to persist renewed credentials: {e}");
                            } else {
                                tracing::info!("[RUNNER {id_clone}] Renewed credentials persisted to config");
                            }
                        });
                    }
                    clob_client = Some(new_client);
                }
                if let Some(mut order) = order_result {
                    // ── Stop-loss monitor ──────────────────────────────────────────
                    let client_ref = clob_client.as_deref();
                    if let Some(sl_pct) = config.stop_loss_pct {
                        if sl_pct > 0.0 {
                            let ep = order.entry_price.unwrap_or(0.5).max(0.001);
                            let stopped = monitor_stop_loss(
                                &id, client_ref, &store, &mut order, sl_pct, next_window,
                            ).await;
                            if stopped {
                                // Mark result immediately — resolution logic will skip this order
                                let exit_p = order.entry_price.unwrap_or(ep);
                                order.result = Some("STOP".to_string());
                                order.pnl = Some(order.amount_usdc * (exit_p / ep - 1.0));
                                order.stop_loss_triggered = true;
                            }
                        }
                    }
                    live_orders.push(order);
                    store.persist();
                }
            }

            // Metrics from backtest (historical) are still useful for display
            let metrics = eval_polymarket(&script_content, &buffer, window_minutes, &config);
            let wallet_balance = if let Some(ref client) = clob_client {
                // Live mode: read real USDC balance from the CLOB/exchange
                fetch_usdc_balance_clob(client).await
            } else {
                // Paper mode: running balance = initial + sum of all settled pnl.
                // Previously this returned `initial_balance` verbatim, so the UI
                // showed a stuck $1000. Now it reflects the paper P&L so the
                // user sees their strategy's realized P&L in the Wallet widget.
                let settled_pnl: f64 = live_orders.iter().filter_map(|o| o.pnl).sum();
                Some(config.initial_balance + settled_pnl)
            };
            update_runner_result(
                &store, &id, &config, &metrics, None,
                config.wallet_address.clone(),
                wallet_balance,
                Some(live_orders.clone()),
                Some(live_wins),
                Some(live_total_trades),
                Some(current_signal),
            ).await;
            store.persist();
        }

        // Update live feed on every 1m candle (not just window boundaries)
        let mut live_feed = None;
            if let Some(series_id) = &config.series_id {
                if let Some(series) = crate::tools::series::builtin_series().into_iter().find(|s| s.id == *series_id) {
                    let market_slug = format!("{}-{}", series.slug_prefix, current_window);

                    let price_to_beat = buffer.iter()
                        .filter(|c| c.open_time_ms >= current_window as i64 * 1000)
                        .min_by_key(|c| c.open_time_ms)
                        .map(|c| c.open)
                        .unwrap_or(live.open);

                    let (yes_token_price, no_token_price) =
                        match (&config.poly_token_id, &config.poly_no_token_id) {
                            (Some(yes), Some(no)) => fetch_token_prices(yes, no).await,
                            _ => (0.0, 0.0),
                        };

                    // Use Chainlink price if available, otherwise Binance candle close
                    let current_btc_price = if let Some(ref cl) = chainlink_price {
                        let cl_price = *cl.read().await;
                        if let Some(p) = cl_price {
                            tracing::debug!("[RUNNER {id}] Using Chainlink price: {p}");
                            p
                        } else {
                            live.close
                        }
                    } else {
                        live.close
                    };

                    live_feed = Some(LiveFeedData {
                        current_btc_price,
                        market_slug,
                        window_timestamp: current_window,
                        window_seconds_left,
                        price_to_beat,
                        yes_token_price,
                        no_token_price,
                        price_history: price_history.iter().cloned().collect(),
                    });
                }
            }

        // Update live_feed in result without re-running metrics
        let mut map = store.runners.lock().unwrap();
        if let Some(r) = map.get_mut(&id) {
            if let Some(ref mut result) = r.result {
                result.live_feed = live_feed;
            }
        }
    }
    // If we left the loop because the watchdog tripped, mark the runner as
    // errored so the dashboard surfaces it and (if auto_restart=true) the
    // daemon respawns a fresh loop with a fresh WS connection.
    if watchdog_tripped {
        let mut map = store.runners.lock().unwrap();
        if let Some(r) = map.get_mut(&id) {
            r.status.status = "error".to_string();
            r.status.error = Some(format!(
                "Watchdog stopped runner: no WS candle for >{}s", WATCHDOG_STALL_SECS
            ));
        }
    }
    tracing::info!("[RUNNER {id}] Polymarket feed closed, exiting (watchdog_tripped={})", watchdog_tripped);
}

/// Resolve both YES and NO token IDs for the current window slug.
async fn resolve_token_for_window(series_id: &str, window_ts: u64) -> anyhow::Result<(String, String, String)> {
    // Retry with backoff: network blips to Gamma API are common during short
    // connectivity outages. 3 attempts with 1.5s/3s/4.5s backoff covers ~10s
    // of transient issues before returning an error the caller can handle.
    const MAX_ATTEMPTS: u32 = 3;
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 1..=MAX_ATTEMPTS {
        match resolve_token_for_window_once(series_id, window_ts).await {
            Ok(v) => return Ok(v),
            Err(e) => {
                tracing::warn!(
                    "[RESOLVE] Attempt {}/{} failed for window {}: {}",
                    attempt, MAX_ATTEMPTS, window_ts, e
                );
                last_err = Some(e);
                if attempt < MAX_ATTEMPTS {
                    let delay_ms = 1500 * attempt as u64;
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("resolve_token_for_window exhausted retries")))
}

async fn resolve_token_for_window_once(series_id: &str, window_ts: u64) -> anyhow::Result<(String, String, String)> {
    let series = crate::tools::series::builtin_series()
        .into_iter()
        .find(|s| s.id == series_id)
        .ok_or_else(|| anyhow::anyhow!("Selected Market Series is not recognized: {}", series_id))?;

    // Polymarket recurrent markets may use seconds or milliseconds in the slug.
    // Try seconds first, then milliseconds as fallback.
    let slug_seconds = format!("{}-{}", series.slug_prefix, window_ts);
    let slug_millis  = format!("{}-{}", series.slug_prefix, window_ts * 1000);

    tracing::info!(
        "[RESOLVE] Trying slug '{}' (seconds) for window {}",
        slug_seconds, window_ts
    );

    let market = match polymarket_trader::markets::get_market(&slug_seconds).await {
        Ok(m) => m,
        Err(e1) => {
            tracing::info!(
                "[RESOLVE] Slug '{}' not found ({}). Trying milliseconds fallback '{}'...",
                slug_seconds, e1, slug_millis
            );
            polymarket_trader::markets::get_market(&slug_millis)
                .await
                .map_err(|e2| anyhow::anyhow!(
                    "No active Polymarket market found for slugs {} or {}. Errors: {} | {}",
                    slug_seconds, slug_millis, e1, e2
                ))?
        }
    };

    if market.yes_token_id.trim().is_empty() {
        anyhow::bail!("The selected market has no YES token yet for slug {}.", market.slug);
    }
    if market.no_token_id.trim().is_empty() {
        anyhow::bail!("The selected market has no NO token yet for slug {}.", market.slug);
    }

    tracing::info!(
        "[RESOLVE] Resolved tokens for slug '{}': YES={} NO={}",
        market.slug, market.yes_token_id, market.no_token_id
    );

    Ok((market.yes_token_id, market.no_token_id, market.condition_id))
}

/// Monitor an open position for stop-loss between decision and resolution.
/// Polls token price every 10 seconds until `resolution_ts - 5s`.
/// If price drops below `entry_price * (1 - stop_loss_pct)`, places a market
/// sell order and returns `true`. Returns `false` if position held to resolution.
async fn monitor_stop_loss(
    id: &str,
    client: Option<&polymarket_trader::orders::ClobClient>,
    store: &Arc<StrategyRunnerStore>,
    order: &mut LiveOrder,
    stop_loss_pct: f64,
    resolution_ts: i64,
) -> bool {
    use polymarket_trader::orders::Side;

    let entry_price = match order.entry_price {
        Some(p) if p > 0.0 => p,
        _ => return false,
    };
    let stop_price = entry_price * (1.0 - stop_loss_pct);
    let shares = (order.amount_usdc / entry_price).round().max(1.0);
    let is_yes = order.side.starts_with("yes");

    append_runner_log(
        store, id,
        &format!(
            "Stop-loss active: entry={:.4} stop={:.4} ({:.0}% drop) shares={:.0}",
            entry_price, stop_price, stop_loss_pct * 100.0, shares
        ),
    );

    let poll_interval = std::time::Duration::from_secs(10);
    let deadline = resolution_ts - 5; // stop checking 5s before resolution

    loop {
        tokio::time::sleep(poll_interval).await;

        let now = chrono::Utc::now().timestamp();
        if now >= deadline {
            break;
        }

        // Fetch current token price
        let http = reqwest::Client::new();
        let side_str = if is_yes { "buy" } else { "buy" }; // we hold the token, price to sell
        let price_url = format!(
            "https://clob.polymarket.com/price?token_id={}&side={}",
            order.token_id, side_str
        );
        let current_price = match http
            .get(&price_url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
        {
            Ok(resp) => {
                if let Ok(body) = resp.text().await {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                        v["price"].as_str()
                            .and_then(|s| s.parse::<f64>().ok())
                            .unwrap_or(entry_price)
                    } else {
                        entry_price
                    }
                } else {
                    entry_price
                }
            }
            Err(_) => entry_price,
        };

        tracing::debug!(
            "[RUNNER {id}] Stop-loss poll: token={} current={:.4} stop={:.4}",
            order.token_id, current_price, stop_price
        );

        if current_price <= stop_price {
            // Exit: sell the token at market
            let sell_price = (current_price * 0.97).max(0.01); // slightly below bid
            tracing::warn!(
                "[RUNNER {id}] STOP-LOSS triggered: price={:.4} <= stop={:.4}. Selling {:.0} shares.",
                current_price, stop_price, shares
            );
            append_runner_log(
                store, id,
                &format!(
                    "STOP-LOSS: price={:.4} ≤ stop={:.4} — selling {:.0} shares",
                    current_price, stop_price, shares
                ),
            );

            if let Some(client) = client {
                match client.create_limit_order(&order.token_id, Side::Sell, sell_price, shares).await {
                    Ok(resp) => {
                        append_runner_log(
                            store, id,
                            &format!(
                                "Stop-loss sell placed: {:.0} shares @ {:.4} (id={})",
                                shares, sell_price, resp.order_id
                            ),
                        );
                        // Update entry_price to the actual exit price for P&L
                        order.entry_price = Some(current_price);
                    }
                    Err(e) => {
                        append_runner_log(
                            store, id,
                            &format!("Stop-loss sell FAILED: {}", e),
                        );
                        // FIX: record exit price even on failure so P&L reflects the loss
                        order.entry_price = Some(current_price);
                    }
                }
            } else {
                // Paper mode: simulate stop-loss exit
                append_runner_log(
                    store, id,
                    &format!(
                        "Paper stop-loss: exited @ {:.4} (shares={:.0})",
                        current_price, shares
                    ),
                );
                order.entry_price = Some(current_price);
            }
            return true;
        }
    }

    false
}

/// Returns the placed order (if any) and optionally a renewed ClobClient when
/// credentials were refreshed due to order_version_mismatch.
/// Look up a window's Polymarket resolution by reconstructing its slug from
/// the series_id and querying Gamma `/markets`. Returns Some(true) for YES/UP,
/// Some(false) for NO/DOWN, and None if the market hasn't resolved yet or the
/// slug couldn't be resolved.
///
/// This replaces the legacy `resolve_window_outcome` Binance-candle path for
/// Polymarket settlement — the chain settles via Chainlink, and Polymarket's
/// `outcomePrices` reflects the on-chain truth.
async fn fetch_polymarket_resolution(
    series_id: &str,
    window_ts: i64,
    condition_id: Option<&str>,
) -> Option<bool> {
    // Primary path: resolve by condition_id via the CLOB. Robust because the runner
    // captured this id at order time — no slug guessing. The legacy `{prefix}-{ts}`
    // Gamma slug stopped resolving for recurring markets, which silently broke official
    // settlement for every non-BTC runner (they got stuck on binance_provisional).
    if let Some(cid) = condition_id.filter(|c| !c.is_empty()) {
        if let Ok(Some(yes_won)) =
            polymarket_trader::markets::get_resolution_by_condition_id(cid).await
        {
            return Some(yes_won);
        }
    }

    // Fallback: legacy slug lookup (kept for older orders without a stored condition_id).
    let series = crate::tools::series::builtin_series()
        .into_iter()
        .find(|s| s.id == series_id)?;
    let slug_seconds = format!("{}-{}", series.slug_prefix, window_ts);
    let slug_millis  = format!("{}-{}", series.slug_prefix, window_ts * 1000);

    for slug in [&slug_seconds, &slug_millis] {
        match polymarket_trader::markets::get_market_resolution(slug).await {
            Ok(res) if res.closed && res.yes_won.is_some() => {
                return res.yes_won;
            }
            Ok(_) => continue, // slug found but not yet resolved
            Err(_) => continue, // try the other slug shape
        }
    }
    None
}

/// Spawn a background task that polls Polymarket's Gamma API until the official
/// resolution for `window_ts` is available, then updates the matching LiveOrder
/// in the store and reconciles the P&L.
///
/// Rationale: the Chainlink oracle that settles Polymarket binary markets
/// typically publishes 60–180 seconds after the window closes.  At the moment
/// the live runner settles a position (first tick of the next window), the
/// market is often still "open" on Gamma.  Without this monitor, runners fall
/// back to Binance price comparison which may not match the oracle.
///
/// The task polls every 30 s, gives up after 10 min, and replaces the Binance
/// result only when the official resolution is found.
fn spawn_resolution_monitor(
    runner_id: String,
    series_id: String,
    window_ts: i64,
    store: Arc<StrategyRunnerStore>,
    binance_fallback_yes_won: bool,
    // For tick_runner_loop: the shared internal state so balance + win counter
    // can be corrected in-place (not just RunnerResult which the loop overwrites).
    tick_state: Option<Arc<std::sync::Mutex<TickRunnerState>>>,
    // Fee % applied to winning payout — must match the settle-time fee model.
    fee_pct: f64,
) {
    tokio::spawn(async move {
        let id = runner_id.as_str();
        // 30-minute window — recurring crypto markets are often marked closed on the
        // CLOB later than 10 min after window close, which previously left orders stuck
        // on binance_provisional. The periodic spawn_resolution_sweep catches anything
        // that resolves even later than this.
        let deadline = chrono::Utc::now().timestamp() + 1800;

        let mut interval = tokio::time::interval(std::time::Duration::from_secs(45));
        interval.tick().await; // skip immediate first tick — already tried once

        loop {
            interval.tick().await;

            if chrono::Utc::now().timestamp() > deadline {
                tracing::debug!(
                    "[RUNNER {id}] Resolution monitor: giving up on window {window_ts} after 10 min"
                );
                return;
            }

            // Pull the order's stored condition_id (set at order time) for the robust
            // CLOB-by-condition lookup; falls back to the legacy slug when absent.
            let order_cid: Option<String> = {
                let map = store.runners.lock().unwrap();
                map.get(id)
                    .and_then(|r| r.result.as_ref())
                    .and_then(|res| res.live_orders.iter().find(|o| o.window_ts == window_ts))
                    .and_then(|o| o.condition_id.clone())
            };
            if let Some(yes_won) =
                fetch_polymarket_resolution(&series_id, window_ts, order_cid.as_deref()).await
            {
                // Official resolution arrived — patch the matching order in the store
                let already_correct = {
                    let map = store.runners.lock().unwrap();
                    map.get(id)
                        .and_then(|r| r.result.as_ref())
                        .and_then(|res| res.live_orders.iter().find(|o| o.window_ts == window_ts))
                        .map(|o| o.resolution_source.as_deref() == Some("polymarket"))
                        .unwrap_or(false)
                };
                if already_correct { return; }

                // Recalculate P&L with the official resolution. Settle on the same price
                // basis as the provisional settle + sweep (book-VWAP fill when sane, else
                // entry) — see `settle_price`. Using raw entry here inflated win P&L.
                let (price, stake, position, old_pnl) = {
                    let map = store.runners.lock().unwrap();
                    map.get(id)
                        .and_then(|r| r.result.as_ref())
                        .and_then(|res| res.live_orders.iter().find(|o| o.window_ts == window_ts))
                        .map(|o| (
                            settle_price(o.entry_price, o.fill_price),
                            o.amount_usdc,
                            if o.side.starts_with("yes") { 1i32 } else { -1i32 },
                            o.pnl.unwrap_or(0.0),
                        ))
                        .unwrap_or((0.5, 0.0, 0, 0.0))
                };

                let won = (position == 1 && yes_won) || (position == -1 && !yes_won);
                // Apply fee_pct to the winning payout (parity with settle-time + backtest).
                let pnl = if won && price > 0.0 {
                    (stake / price) * (1.0 - fee_pct / 100.0) - stake
                } else {
                    -stake
                };

                let mut changed = false;
                {
                    let mut map = store.runners.lock().unwrap();
                    if let Some(r) = map.get_mut(id) {
                        if let Some(ref mut res) = r.result {
                            if let Some(order) = res.live_orders.iter_mut().find(|o| o.window_ts == window_ts) {
                                changed = order.resolution_yes_won != Some(yes_won);
                                order.resolution_yes_won = Some(yes_won);
                                order.resolution_source = Some("polymarket".to_string());
                                order.result = Some(if won { "WIN".to_string() } else { "LOSS".to_string() });

                                if changed {
                                    let pnl_delta = pnl - old_pnl;
                                    order.pnl = Some(pnl);

                                    // on_candle: do NOT write res.balance here. The loop and
                                    // the sweep both recompute balance = initial + sum(pnl)
                                    // (single source of truth); a third incremental writer
                                    // here is what made the dashboard P&L flicker between the
                                    // provisional and official values during convergence.
                                    // The tick engine's balance is corrected via tick_state below.
                                    //
                                    // live_wins counter: adjust if win/loss flipped (the loop
                                    // recomputes it for on_candle; this keeps tick consistent).
                                    let was_win = binance_fallback_yes_won == (position == 1);
                                    let is_win  = won;
                                    if was_win && !is_win {
                                        res.live_wins = res.live_wins.saturating_sub(1);
                                    } else if !was_win && is_win {
                                        res.live_wins = res.live_wins.saturating_add(1);
                                    }
                                    tracing::info!(
                                        "[RUNNER {id}] Official resolution for window {window_ts}: \
                                        yes_won={yes_won} (was binance={binance_fallback_yes_won}) \
                                        pnl_delta={pnl_delta:+.2}"
                                    );
                                }
                            }
                        }
                    }
                }

                // For tick_runner_loop: patch the internal TickRunnerState so the loop's
                // `res.balance = TickRunnerState.balance` write propagates the corrected value.
                if changed {
                    if let Some(ref ts) = tick_state {
                        let mut s = ts.lock().unwrap();
                        // Remove provisional pnl and credit official pnl (settle price basis)
                        let provisional_pnl = if binance_fallback_yes_won == (position == 1) {
                            // was a win: payout was stake/price
                            if price > 0.0 { stake / price } else { 0.0 }
                        } else {
                            0.0 // was a loss: nothing was added
                        };
                        let official_pnl = if won && price > 0.0 { stake / price } else { 0.0 };
                        s.balance = s.balance - provisional_pnl + official_pnl;
                    }
                }

                store.persist();

                append_runner_log(
                    &store, id,
                    &format!(
                        "Resolution updated: window {window_ts} → {} (official Polymarket oracle, was Binance provisional{})",
                        if yes_won { "YES/UP" } else { "NO/DOWN" },
                        if changed { " — balance corrected" } else { ", no change" }
                    ),
                );
                return; // done — official resolution recorded
            }
        }
    });
}

/// Global periodic re-resolution sweep. The per-order `spawn_resolution_monitor` gives
/// up after a fixed window, but the CLOB sometimes marks recurring crypto markets
/// resolved later than that, which left orders stuck on the unreliable
/// `binance_provisional` resolution — making Dry Run / live P&L diverge from the real
/// Polymarket oracle. This task retries the official CLOB-by-condition_id resolution for
/// ANY still-provisional order that carries a condition_id, with no time limit, so the
/// books converge to the official outcome.
///
/// Scoped to on_candle runners: the tick engine owns its balance in `TickRunnerState`,
/// which the loop overwrites every second, so patching it out-of-band here would be
/// clobbered. on_candle balance is derived from `sum(order.pnl)` each loop cycle, so
/// patching `order.pnl` (and the intermediate `res.balance`) is safe and converges.
pub fn spawn_resolution_sweep(store: Arc<StrategyRunnerStore>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        interval.tick().await; // skip the immediate first tick
        loop {
            interval.tick().await;

            // Snapshot the provisional on_candle orders that have a condition_id.
            let todo: Vec<(String, i64, String, f64)> = {
                let map = store.runners.lock().unwrap();
                let mut v = Vec::new();
                for (id, r) in map.iter() {
                    if r.config.kind.as_deref() == Some("rhai_tick") { continue; }
                    let fee = r.config.fee_pct;
                    if let Some(res) = r.result.as_ref() {
                        for o in &res.live_orders {
                            if o.resolution_source.as_deref() != Some("polymarket")
                                && o.pnl.is_some()
                                && o.result.is_some()
                            {
                                if let Some(cid) = o.condition_id.as_ref().filter(|c| !c.is_empty()) {
                                    v.push((id.clone(), o.window_ts, cid.clone(), fee));
                                }
                            }
                        }
                    }
                }
                // Oldest window first — those are the most likely to be resolved on the
                // CLOB already, so resolvable orders get patched before the per-cycle cap
                // is spent. Without this, late-iteration runners (e.g. DOGE/BNB) starved:
                // the cap was always consumed by other runners' fresher orders and theirs
                // were never reached.
                v.sort_by_key(|(_, wts, _, _)| *wts);
                v
            };
            if todo.is_empty() { continue; }

            let mut patched = 0usize;
            // Cap raised to cover the realistic resolvable backlog in one pass (most
            // entries return None — not yet closed — and are retried next cycle).
            for (id, window_ts, cid, fee) in todo.into_iter().take(400) {
                let yes_won = match polymarket_trader::markets::get_resolution_by_condition_id(&cid).await {
                    Ok(Some(v)) => v,
                    _ => continue, // not settled yet, or lookup failed — retry next cycle
                };
                let mut map = store.runners.lock().unwrap();
                let Some(r) = map.get_mut(&id) else { continue; };
                let initial = r.config.initial_balance;
                let Some(res) = r.result.as_mut() else { continue; };
                {
                    let Some(order) = res.live_orders.iter_mut().find(|o| o.window_ts == window_ts) else { continue; };
                    if order.resolution_source.as_deref() == Some("polymarket") { continue; }
                    let stake = order.amount_usdc;
                    let position = if order.side.starts_with("yes") { 1 } else { -1 };
                    let won = (position == 1 && yes_won) || (position == -1 && !yes_won);
                    // Settle on the SAME price the provisional settle used — the book-VWAP
                    // fill when sane, else the decision entry. Using entry_price here (the
                    // cheaper top-of-book ask) inflated every win's P&L vs the realistic
                    // fill, which is what made the balance jump up on re-resolution.
                    let price = settle_price(order.entry_price, order.fill_price);
                    let pnl = if won && price > 0.0 {
                        (stake / price) * (1.0 - fee / 100.0) - stake
                    } else {
                        -stake
                    };
                    order.resolution_yes_won = Some(yes_won);
                    order.resolution_source = Some("polymarket".to_string());
                    order.result = Some(if won { "WIN".to_string() } else { "LOSS".to_string() });
                    order.pnl = Some(pnl);
                }
                // Single source of truth: every balance writer recomputes from sum(pnl)
                // so the loop / sweep / monitor never disagree (the cause of the UI flicker
                // between the provisional and official P&L during resolution convergence).
                res.balance = initial + res.live_orders.iter().filter_map(|o| o.pnl).sum::<f64>();
                res.live_wins = res.live_orders.iter()
                    .filter(|o| o.result.as_deref().map(|s| s.starts_with("WIN")).unwrap_or(false))
                    .count() as u32;
                patched += 1;
            }
            if patched > 0 {
                store.persist();
                tracing::info!("[RESOLUTION-SWEEP] upgraded {patched} provisional orders to official resolution");
            }
        }
    });
}

/// Global portfolio guard loop. Every 5 minutes, reads the REAL Polymarket wallet
/// balance (via any live runner's CLOB creds) and stops ALL live runners if the wallet
/// has dropped more than `max_loss_pct` from its baseline. This is the cross-runner
/// safety net that was missing during the May incident: individual runners each stayed
/// "within their own limit" while the aggregate drained the wallet. The guard watches
/// the WALLET, not any single runner.
///
/// `max_loss_pct`: 0.5 = halt at -50%. Set to 0 to disable. Baseline = first observed
/// balance once at least one live runner exists.
pub fn spawn_portfolio_guard(store: Arc<StrategyRunnerStore>, max_loss_pct: f64) {
    if max_loss_pct <= 0.0 {
        tracing::info!("[PORTFOLIO_GUARD] disabled (max_loss_pct=0)");
        return;
    }
    tokio::spawn(async move {
        let guard = crate::portfolio_guard::PortfolioGuard::new(max_loss_pct);
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        interval.tick().await;
        loop {
            interval.tick().await;

            // Grab CLOB creds from any running live runner.
            let creds = {
                let map = store.runners.lock().unwrap();
                map.values()
                    .find(|r| r.config.mode == "live"
                        && r.status.status == "running"
                        && r.config.poly_creds.is_some())
                    .and_then(|r| r.config.poly_creds.clone())
            };
            let Some(creds) = creds else { continue; }; // no live runner → nothing to guard

            let client = polymarket_trader::orders::ClobClient::new(creds);
            let Some(balance) = fetch_usdc_balance_clob(&client).await else { continue; };

            guard.set_baseline(balance); // no-op after first set
            if guard.check(balance) {
                let stopped = store.stop_all_live();
                store.persist();
                tracing::error!(
                    "[PORTFOLIO_GUARD] WALLET DROP BREACH at ${:.2} — stopped {} live runners",
                    balance, stopped.len()
                );
            }
        }
    });
}

/// Poll the CLOB `/data/trades` endpoint to find the on-chain fills for the
/// freshly-placed order and return (vwap_fill_price, total_size, first_tx_hash).
///
/// The order endpoint only returns `{order_id, status}` — the real fill price
/// only appears in the trade-history endpoint after the match settles. We poll
/// briefly (up to ~6s) so the live runner can stamp the LiveOrder with the
/// actual price the chain paid, instead of the decision-time midpoint.
///
/// Returns None when no matching trade shows up within the polling window
/// (e.g. order didn't fill, or the trade endpoint is lagging — the historical
/// backfill tool will reconcile it later).
async fn reconcile_order_fill(
    client: &polymarket_trader::orders::ClobClient,
    order_id: &str,
    after_secs: i64,
) -> Option<(f64, f64, String)> {
    let _permit = clob_semaphore().acquire_owned().await;
    if order_id.is_empty() || order_id == "ok" || order_id == "FAILED" || order_id == "REJECTED" {
        return None;
    }
    // Poll up to 6 times with a 1s gap. CLOB usually surfaces matched trades
    // within ~1-2 seconds of the order being posted.
    for attempt in 0..6 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        let trades = match client.get_trade_history(None, Some(after_secs - 5), None).await {
            Ok(t) => t,
            Err(e) => {
                tracing::debug!("[FILL] /data/trades poll attempt {attempt} failed: {e}");
                continue;
            }
        };
        let matched: Vec<_> = trades.iter()
            .filter(|t| t.taker_order_id.eq_ignore_ascii_case(order_id))
            .collect();
        if matched.is_empty() {
            continue;
        }
        let total_size: f64 = matched.iter().map(|t| t.size).sum();
        if total_size <= 0.0 {
            continue;
        }
        let weighted: f64 = matched.iter().map(|t| t.size * t.price).sum();
        let vwap = weighted / total_size;
        let tx_hash = matched.first().map(|t| t.transaction_hash.clone()).unwrap_or_default();
        return Some((vwap, total_size, tx_hash));
    }
    None
}

/// Spawn a background task that:
/// 1. Polls `/data/trades` for up to ~120 s after order placement to capture
///    late fills (market orders that settle slowly, limit orders that fill later).
/// 2. When a fill is found, updates the LiveOrder in the store in-place so the
///    UI shows the real fill_price / fill_size / tx_hash instead of the midpoint.
/// 3. For limit orders (`is_limit = true`): if still unfilled when
///    `window_close_ts` passes, cancels the order via the CLOB API and updates
///    status to "CANCELLED" so P&L doesn't get credited on a ghost position.
fn spawn_order_monitor(
    runner_id: String,
    order_id: String,
    client: Arc<polymarket_trader::orders::ClobClient>,
    store: Arc<StrategyRunnerStore>,
    placed_at: i64,
    window_close_ts: i64,
    is_limit: bool,
) {
    tokio::spawn(async move {
        let id = runner_id.as_str();
        // Give the chain / matching engine up to 120 s.
        // Poll every 10 s so we don't hammer the API.
        let deadline = placed_at + 120;
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
        interval.tick().await; // skip the immediate first tick (already checked inline)

        loop {
            interval.tick().await;
            let now = chrono::Utc::now().timestamp();

            // Try to find a fill in trade history.
            if let Some((vwap, size, tx)) = reconcile_order_fill(&client, &order_id, placed_at - 5).await {
                tracing::info!(
                    "[RUNNER {id}] BG-FILL order {}: price={:.4} size={:.2} tx={}",
                    order_id, vwap, size, tx
                );
                // Update the matching LiveOrder in the store.
                let mut map = store.runners.lock().unwrap();
                if let Some(r) = map.get_mut(id) {
                    if let Some(ref mut res) = r.result {
                        if let Some(o) = res.live_orders.iter_mut().find(|o| o.order_id == order_id) {
                            o.fill_price = Some(vwap);
                            o.fill_size  = Some(size);
                            o.tx_hash    = Some(tx.clone());
                            o.status     = "MATCHED".to_string();
                        }
                    }
                }
                drop(map);
                append_runner_log(
                    &store, id,
                    &format!(
                        "Fill confirmed (bg): order {} filled @ {:.4} ({:.2} shares) tx={}",
                        order_id, vwap, size, tx
                    ),
                );
                store.persist();
                return;
            }

            // Cancel limit orders once window has closed and they're still open.
            if is_limit && now >= window_close_ts {
                tracing::info!("[RUNNER {id}] BG-CANCEL limit order {} (window closed, unfilled)", order_id);
                if let Err(e) = client.cancel_order(&order_id).await {
                    tracing::warn!("[RUNNER {id}] BG-CANCEL failed for {}: {e}", order_id);
                }
                // Mark as cancelled in the store so P&L isn't counted.
                let mut map = store.runners.lock().unwrap();
                if let Some(r) = map.get_mut(id) {
                    if let Some(ref mut res) = r.result {
                        if let Some(o) = res.live_orders.iter_mut().find(|o| o.order_id == order_id) {
                            o.status = "CANCELLED".to_string();
                            o.result = Some("CANCELLED".to_string());
                            o.pnl    = Some(0.0);
                        }
                    }
                }
                drop(map);
                append_runner_log(
                    &store, id,
                    &format!("Limit order {} cancelled (unfilled at window close)", order_id),
                );
                store.persist();
                return;
            }

            if now > deadline {
                tracing::debug!("[RUNNER {id}] BG-FILL: giving up on order {} after 120 s", order_id);
                return;
            }
        }
    });
}

async fn execute_live_polymarket_signal(
    id: &str,
    client: Option<Arc<polymarket_trader::orders::ClobClient>>,
    signal: &str,
    script_frac: f64,
    config: &RunnerConfig,
    _live: &crate::live_feed::LiveCandle,
    store: &Arc<StrategyRunnerStore>,
    window_ts: i64,
    window_close_ts: i64,
    yes_token_price: f64,
    no_token_price: f64,
    kelly_mult: f64,
    series_id: &str,
) -> (Option<LiveOrder>, Option<Arc<polymarket_trader::orders::ClobClient>>) {
    // Serialize order execution per series to prevent book impact from parallel runners
    let _series_permit = if !series_id.is_empty() {
        let sem = get_series_semaphore(series_id);
        Some(sem.acquire_owned().await)
    } else {
        None
    };
    use polymarket_trader::orders::Side;

    // In binary markets YES/NO are complementary tokens.
    // "yes" → buy YES token  |  "no" → buy NO token
    // Distinguish live vs paper before resolving token_id so the skip reason
    // is actionable: live-mode skip on missing token_id = resolution failure;
    // paper-mode skip = programmer error (signal without token configured).
    let is_live_order = client.is_some();

    let (token_id, side) = if signal.starts_with("yes") {
        match &config.poly_token_id {
            Some(tid) if !tid.is_empty() => (tid.clone(), Side::Buy),
            _ => {
                let reason = if is_live_order {
                    format!(
                        "LIVE SKIP window {window_ts}: YES token_id not resolved — \
                        Gamma API may be lagging. Order NOT placed. \
                        Check runner logs for token resolution errors."
                    )
                } else {
                    "Paper SKIP: YES token_id not set".to_string()
                };
                tracing::error!("[RUNNER {id}] {reason}");
                append_runner_log(store, id, &reason);
                return (None, None);
            }
        }
    } else if signal.starts_with("no") {
        match &config.poly_no_token_id {
            Some(tid) if !tid.is_empty() => (tid.clone(), Side::Buy),
            _ => {
                let reason = if is_live_order {
                    format!(
                        "LIVE SKIP window {window_ts}: NO token_id not resolved — \
                        Gamma API may be lagging. Order NOT placed. \
                        Check runner logs for token resolution errors."
                    )
                } else {
                    "Paper SKIP: NO token_id not set".to_string()
                };
                tracing::error!("[RUNNER {id}] {reason}");
                append_runner_log(store, id, &reason);
                return (None, None);
            }
        }
    } else {
        tracing::debug!("[RUNNER {id}] Signal '{signal}' — no order placed");
        return (None, None);
    };

    // ── Position sizing ────────────────────────────────────────────
    let (_balance, amount_usdc) = match client.as_deref() {
        Some(c) => {
            let bal = fetch_usdc_balance_clob(c).await.unwrap_or(0.0);
            if bal <= 0.0 {
                tracing::warn!("[RUNNER {id}] Cannot place order: zero or unknown USDC/pUSD balance");
                append_runner_log(store, id, "Skipped: zero USDC/pUSD balance");
                return (None, None);
            }
            let amt = match config.live_sizing_mode {
                LiveSizingMode::Fixed => {
                    let amt = config.live_sizing_value.max(5.0).round();
                    tracing::info!("[RUNNER {id}] Sizing (fixed): ${:.0}", amt);
                    amt
                }
                LiveSizingMode::Percent => {
                    // Use runner's initial_balance as sizing base (NOT the shared wallet balance)
                    // to isolate each runner's capital allocation.
                    let sizing_base = config.initial_balance.min(bal);
                    let max_frac = (config.live_sizing_value / 100.0).max(0.0).min(1.0);
                    let frac = script_frac.clamp(0.0, max_frac);
                    let amt = (sizing_base * frac).max(5.0).round();
                    tracing::info!(
                        "[RUNNER {id}] Sizing (percent): sizing_base=${:.2} (initial={:.0}, wallet={:.2}) script_frac={:.4} max_frac={:.4} amount=${:.0}",
                        sizing_base, config.initial_balance, bal, script_frac, max_frac, amt
                    );
                    amt
                }
            };
            // Apply confidence-weighted multiplier from the script's kelly_size.
            // Cap at runner's initial_balance and available wallet balance.
            let amt_scaled = (amt * kelly_mult).max(5.0).round().min(config.initial_balance).min(bal);
            if (kelly_mult - 1.0).abs() > 0.001 {
                tracing::info!(
                    "[RUNNER {id}] Kelly multiplier {:.2}x: amount {} → {}",
                    kelly_mult, amt, amt_scaled
                );
            }
            (bal, amt_scaled)
        }
        None => {
            // Paper mode: use initial_balance as simulated balance
            let bal = config.initial_balance;
            let amt = match config.live_sizing_mode {
                LiveSizingMode::Fixed => {
                    let amt = config.live_sizing_value.max(5.0).round().min(bal);
                    tracing::info!("[RUNNER {id}] Paper sizing (fixed): ${:.0}", amt);
                    amt
                }
                LiveSizingMode::Percent => {
                    // live_sizing_value is stored as 0–100 (e.g. 5 = 5%); convert to 0–1 fraction
                    let max_frac = (config.live_sizing_value / 100.0).max(0.0).min(1.0);
                    let frac = script_frac.clamp(0.0, max_frac);
                    let amt = (bal * frac).max(5.0).round().min(bal);
                    tracing::info!(
                        "[RUNNER {id}] Paper sizing (percent): balance=${:.2} script_frac={:.4} max_frac={:.4} amount=${:.0}",
                        bal, script_frac, max_frac, amt
                    );
                    amt
                }
            };
            // Apply confidence-weighted multiplier (paper mode mirrors live).
            let amt_scaled = (amt * kelly_mult).max(5.0).round().min(bal);
            if (kelly_mult - 1.0).abs() > 0.001 {
                tracing::info!(
                    "[RUNNER {id}] Paper Kelly multiplier {:.2}x: amount {} → {}",
                    kelly_mult, amt, amt_scaled
                );
            }
            if amt_scaled <= 0.0 || amt_scaled > bal {
                tracing::warn!("[RUNNER {id}] Paper order: insufficient balance ${:.2} for amount ${:.0}", bal, amt_scaled);
                append_runner_log(store, id, &format!("Skipped: insufficient paper balance ${:.2}", bal));
                return (None, None);
            }
            (bal, amt_scaled)
        }
    };

    let ep = if signal.starts_with("yes") { yes_token_price } else { no_token_price };

    // Reject if either side of the book shows an extreme price. A midpoint
    // below 0.10 means the book is empty/stale or extremely one-sided.
    // Since we cannot determine the actual fill price (CLOB response lacks it),
    // filling at such extremes leads to wildly incorrect P&L accounting.
    const MIN_VALID_PRICE: f64 = 0.10;
    if yes_token_price < MIN_VALID_PRICE || no_token_price < MIN_VALID_PRICE {
        tracing::warn!(
            "[RUNNER {id}] Skipped: price feed unreliable (yes={:.4} no={:.4})",
            yes_token_price, no_token_price
        );
        append_runner_log(
            store, id,
            &format!(
                "Skipped: price feed unreliable (yes={:.4} no={:.4}); window {}",
                yes_token_price, no_token_price, window_ts
            ),
        );
        return (None, None);
    }

    // Skip if entry price is below minimum (prevents extreme long-shot bets)
    let min_ep = config.min_entry_price;
    if min_ep > 0.0 && ep < min_ep {
        tracing::info!("[RUNNER {id}] Skipped: entry price {:.4} < min_entry_price {:.4}", ep, min_ep);
        append_runner_log(
            store, id,
            &format!("SKIP: ep={:.4} < min_entry_price={:.4} (extreme long-shot bet blocked)", ep, min_ep),
        );
        return (None, None);
    }

    // Skip trade if entry price exceeds the configured maximum
    if let Some(max_ep) = config.max_entry_price {
        if ep > max_ep {
            tracing::info!("[RUNNER {id}] Skipped: entry price {:.4} > max {:.4}", ep, max_ep);
            append_runner_log(
                store, id,
                &format!("Skipped: entry price {:.4} exceeds max {:.4}", ep, max_ep),
            );
            return (None, None);
        }
    }

    if let Some(ref client_arc) = client {
        // ── LIVE mode: place real order via CLOB ──────────────────

        // ── Diagnostic: probe CLOB /price and /book for this token ──
        let diag_client = reqwest::Client::new();
        let price_url = format!("https://clob.polymarket.com/price?token_id={}&side=buy", token_id);
        match diag_client.get(&price_url).timeout(std::time::Duration::from_secs(5)).send().await {
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                tracing::info!("[RUNNER {id}] DIAG /price for token {}: {} | {}", token_id, status, body);
            }
            Err(e) => {
                tracing::warn!("[RUNNER {id}] DIAG /price request failed: {}", e);
            }
        }
        let book_url = format!("https://clob.polymarket.com/book?token_id={}&side=buy", token_id);
        match diag_client.get(&book_url).timeout(std::time::Duration::from_secs(5)).send().await {
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                tracing::info!("[RUNNER {id}] DIAG /book for token {}: {} | {}", token_id, status, body);
            }
            Err(e) => {
                tracing::warn!("[RUNNER {id}] DIAG /book request failed: {}", e);
            }
        }

        // Use the midpoint as worst acceptable price with 5% slippage tolerance.
        // This prevents fills at extreme prices when the book is thin.
        let slippage_cap = config.max_slippage_pct.unwrap_or(5.0) / 100.0;
        let worst_price = (ep * (1.0 + slippage_cap)).min(0.95);
        let max_retries = 3;
        let retry_delay = std::time::Duration::from_secs(10);
        let mut attempt = 0;

        tracing::info!(
            "[RUNNER {id}] Slippage cap: {:.1}% → worst_price={:.4} (mid={:.4})",
            slippage_cap * 100.0, worst_price, ep
        );

        // Holds a freshly-renewed client when order_version_mismatch triggers
        // re-authentication.  Returned to the caller so the runner loop can
        // replace its clob_client reference for all subsequent windows.
        let mut renewed: Option<Arc<polymarket_trader::orders::ClobClient>> = None;

        // Helper: borrow whichever client is currently active.
        // After renewal, all order calls go through the renewed client.
        macro_rules! active {
            () => {
                renewed.as_deref().unwrap_or(client_arc.as_ref())
            };
        }

        loop {
            attempt += 1;
            let is_final = attempt >= max_retries;

            // On the final attempt fall back to a limit order at the decision-time
            // mid price. A limit order only fills if the market comes to you —
            // it is safer than a market order when the book is thin or the window
            // is nearly closed.
            if is_final {
                let limit_price = if signal.starts_with("yes") {
                    (yes_token_price * 100.0).round() / 100.0
                } else {
                    (no_token_price * 100.0).round() / 100.0
                }.max(0.01);
                let shares = (amount_usdc / limit_price).round();
                tracing::info!(
                    "[RUNNER {id}] Live LIMIT order (attempt {}/{}): {:?} {:.0} shares (~${:.0} USDC) on token {} @ {:.4}",
                    attempt, max_retries, side, shares, amount_usdc, token_id, limit_price
                );
                match active!().create_limit_order(&token_id, side, limit_price, shares).await {
                    Ok(resp) => {
                        tracing::info!(
                            "[RUNNER {id}] Limit order placed: id={} status={}",
                            resp.order_id, resp.status
                        );
                        append_runner_log(
                            store, id,
                            &format!("Limit order placed: {} {} USDC @{:.4} (id={})", signal, amount_usdc, limit_price, resp.order_id),
                        );
                        let ep = if signal.starts_with("yes") { yes_token_price } else { no_token_price };
                        let placed_at = chrono::Utc::now().timestamp();
                        let (fill_price, fill_size, tx_hash) =
                            match reconcile_order_fill(active!(), &resp.order_id, placed_at).await {
                                Some((p, s, h)) => {
                                    tracing::info!(
                                        "[RUNNER {id}] Real fill price for limit order {}: ${:.4} ({:.2} shares) tx={}",
                                        resp.order_id, p, s, h
                                    );
                                    (Some(p), Some(s), Some(h))
                                }
                                None => {
                                    // Limit order placed but not yet filled. Background monitor
                                    // will update fill data when it lands.
                                    let monitor_client = renewed.clone().unwrap_or_else(|| client_arc.clone());
                                    spawn_order_monitor(
                                        id.to_string(),
                                        resp.order_id.clone(),
                                        monitor_client,
                                        store.clone(),
                                        placed_at,
                                        window_close_ts,
                                        true, // is_limit: cancel at window close if unfilled
                                    );
                                    (None, None, None)
                                }
                            };
                        return (Some(LiveOrder {
                            timestamp: chrono::Utc::now().to_rfc3339(),
                            window_ts,
                            side: signal.to_string(),
                            token_id,
                            amount_usdc,
                            order_id: resp.order_id,
                            status: resp.status,
                            entry_price: Some(ep),
                            result: None,
                            pnl: None,
                            stop_loss_triggered: false,
                            fill_price,
                            fill_size,
                            tx_hash,
                            condition_id: config.poly_condition_id.clone(),
                            ..Default::default()
                        }), renewed);
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        tracing::warn!(
                            "[RUNNER {id}] Limit order failed (attempt {}/{}): {}",
                            attempt, max_retries, msg
                        );
                        append_runner_log(
                            store, id,
                            &format!(
                                "Skipped: order failed for window {} after {} attempts (limit fallback also failed: {})",
                                window_ts, max_retries, msg
                            ),
                        );
                        return (None, renewed);
                    }
                }
            }

            tracing::info!(
                "[RUNNER {id}] Live MARKET order (attempt {}/{}): {:?} ${:.0} USDC on token {} worst_price={:.4}",
                attempt, max_retries, side, amount_usdc, token_id, worst_price
            );

            match active!().create_market_order(&token_id, side, amount_usdc, worst_price).await {
                Ok(resp) => {
                    tracing::info!(
                        "[RUNNER {id}] Order placed: id={} status={}",
                        resp.order_id, resp.status
                    );
                    append_runner_log(
                        store, id,
                        &format!("Order placed: {} {} USDC (id={})", signal, amount_usdc, resp.order_id),
                    );
                    let ep = if signal.starts_with("yes") { yes_token_price } else { no_token_price };
                    let placed_at = chrono::Utc::now().timestamp();
                    let (fill_price, fill_size, tx_hash) =
                        match reconcile_order_fill(active!(), &resp.order_id, placed_at).await {
                            Some((p, s, h)) => {
                                tracing::info!(
                                    "[RUNNER {id}] Real fill price for market order {}: ${:.4} ({:.2} shares) tx={}",
                                    resp.order_id, p, s, h
                                );
                                (Some(p), Some(s), Some(h))
                            }
                            None => {
                                // Market order placed but trade not yet in history.
                                // Background monitor will update fill data when it lands.
                                let monitor_client = renewed.clone().unwrap_or_else(|| client_arc.clone());
                                spawn_order_monitor(
                                    id.to_string(),
                                    resp.order_id.clone(),
                                    monitor_client,
                                    store.clone(),
                                    placed_at,
                                    window_close_ts,
                                    false, // market order — reconcile fill only, don't cancel
                                );
                                (None, None, None)
                            }
                        };
                    return (Some(LiveOrder {
                        timestamp: chrono::Utc::now().to_rfc3339(),
                        window_ts,
                        side: signal.to_string(),
                        token_id,
                        amount_usdc,
                        order_id: resp.order_id,
                        status: resp.status,
                        entry_price: Some(ep),
                        result: None,
                        pnl: None,
                        stop_loss_triggered: false,
                        fill_price,
                        fill_size,
                        tx_hash,
                        condition_id: config.poly_condition_id.clone(),
                        ..Default::default()
                    }), renewed);
                }
                Err(e) => {
                    let msg = e.to_string();
                    tracing::warn!(
                        "[RUNNER {id}] Market order failed (attempt {}/{}): {}",
                        attempt, max_retries, msg
                    );
                    if attempt < max_retries {
                        if msg.contains("order_version_mismatch") {
                            tracing::warn!(
                                "[RUNNER {id}] order_version_mismatch — renewing L2 credentials then falling back to limit order"
                            );
                            match client_arc.renew().await {
                                Ok(new_client) => {
                                    tracing::info!("[RUNNER {id}] Credentials renewed successfully");
                                    append_runner_log(store, id, "Credentials auto-renewed (order_version_mismatch)");
                                    renewed = Some(Arc::new(new_client));
                                }
                                Err(e) => {
                                    tracing::warn!("[RUNNER {id}] Credential renewal failed: {e} — proceeding with limit order fallback");
                                    append_runner_log(store, id, &format!("Credential renewal failed: {e}"));
                                }
                            }
                            attempt = max_retries - 1;
                        } else {
                            tokio::time::sleep(retry_delay).await;
                        }
                        continue;
                    }
                    append_runner_log(
                        store, id,
                        &format!(
                            "Skipped: order failed for window {} after {} attempts: {}",
                            window_ts, max_retries, msg
                        ),
                    );
                    return (None, renewed);
                }
            }
        }
    } else {
        // ── PAPER mode: simulate order with real CLOB market price ───────
        let order_id = format!("paper-{}", chrono::Utc::now().timestamp_millis());

        // Step 1: fetch the real CLOB ask price for this token via GET /price?side=buy.
        // This is the actual price you'd pay as a market buyer — more accurate than mid.
        let clob_ask_price = polymarket_trader::markets::get_market_price(&token_id)
            .await
            .unwrap_or(ep); // fall back to mid on error

        // Step 2: walk the order book to simulate the VWAP for the stake size.
        // This captures the slippage beyond the best ask when the book is thin.
        let (sim_fill_price, slippage_pct) =
            simulate_book_fill(&token_id, amount_usdc, clob_ask_price).await;

        tracing::info!(
            "[RUNNER {id}] Paper order: {} ${:.0} on token {} mid={:.4} ask={:.4} sim_fill={:.4} slippage={:.2}%",
            signal, amount_usdc, token_id, ep, clob_ask_price, sim_fill_price, slippage_pct
        );
        append_runner_log(
            store, id,
            &format!(
                "Paper order: {} ${:.0} mid={:.4} clob_ask={:.4} sim_fill={:.4} slip={:.2}% (id={})",
                signal, amount_usdc, ep, clob_ask_price, sim_fill_price, slippage_pct, order_id
            ),
        );
        (Some(LiveOrder {
            timestamp: chrono::Utc::now().to_rfc3339(),
            window_ts,
            side: signal.to_string(),
            token_id,
            amount_usdc,
            order_id,
            status: "LIVE".to_string(),
            // entry_price = real CLOB ask (what you'd actually pay as a buyer)
            entry_price: Some(clob_ask_price),
            // fill_price = VWAP after walking book depth (captures liquidity cost)
            fill_price: Some(sim_fill_price),
            result: None,
            pnl: None,
            stop_loss_triggered: false,
            condition_id: config.poly_condition_id.clone(),
            ..Default::default()
        }), None)
    }
}

/// Simulate a market-buy fill by walking the ask side of the Polymarket CLOB
/// order book for `token_id`. Returns (vwap_fill_price, slippage_pct).
///
/// On any error (network, parse, empty book) falls back to `mid_price` with
/// 0 % slippage so paper mode still works without a live book.
async fn simulate_book_fill(token_id: &str, amount_usdc: f64, mid_price: f64) -> (f64, f64) {
    let url = format!("https://clob.polymarket.com/book?token_id={}", token_id);
    let client = reqwest::Client::new();
    let body = match client
        .get(&url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => match r.json::<serde_json::Value>().await {
            Ok(v) => v,
            Err(_) => return (mid_price, 0.0),
        },
        _ => return (mid_price, 0.0),
    };

    // The CLOB book endpoint returns {"asks": [{"price":"0.52","size":"150"}, ...], ...}
    // Asks are ordered ascending (lowest ask first). A buy walks them in order.
    let asks = match body.get("asks").and_then(|v| v.as_array()) {
        Some(a) if !a.is_empty() => a.clone(),
        _ => return (mid_price, 0.0),
    };

    let mut remaining = amount_usdc;
    let mut total_shares = 0.0_f64;
    let mut total_cost   = 0.0_f64;

    for level in &asks {
        let price = level.get("price")
            .and_then(|v| v.as_str().map(str::to_string).or_else(|| v.as_f64().map(|f| f.to_string())))
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        let size = level.get("size")
            .and_then(|v| v.as_str().map(str::to_string).or_else(|| v.as_f64().map(|f| f.to_string())))
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);

        if price <= 0.0 || size <= 0.0 { continue; }

        let cost_at_level = price * size; // USD cost to buy all shares at this level
        if cost_at_level >= remaining {
            // Partial fill of this level covers the remainder
            let shares_here = remaining / price;
            total_shares += shares_here;
            total_cost   += remaining;
            break;
        }
        total_shares += size;
        total_cost   += cost_at_level;
        remaining    -= cost_at_level;
    }

    if total_shares <= 0.0 || total_cost <= 0.0 {
        return (mid_price, 0.0);
    }

    let raw_vwap = total_cost / total_shares;

    // ── Sanity-cap the simulated fill ───────────────────────────────────────
    // The NON-favourite token (usually NO on UP/DOWN markets) is illiquid — the
    // crowd trades the YES leg, so the NO `/book` can be nearly empty with a
    // stale resting ask at ~0.99. Walking that book yields a VWAP wildly above
    // the real ask (`mid_price`, from `/price?side=buy`), which then poisons P&L
    // (a $30 NO win at fill 0.99 nets −$0.15 instead of +$30+).
    //
    // A real market buy never fills worse than ask × (1 + max_slippage). Cap the
    // sim to that bound — mirrors the live runner's `max_slippage_pct` reject.
    // When the book disagrees with the ask by more than the cap, the book is
    // unreliable, so we fall back to the capped ask rather than the bogus VWAP.
    const MAX_PAPER_SLIPPAGE_PCT: f64 = 5.0;
    let vwap = if mid_price > 0.0 {
        raw_vwap.min(mid_price * (1.0 + MAX_PAPER_SLIPPAGE_PCT / 100.0))
    } else {
        raw_vwap
    };
    let slippage_pct = if mid_price > 0.0 { (vwap / mid_price - 1.0) * 100.0 } else { 0.0 };
    (vwap, slippage_pct)
}

/// Fetch Yes/No token mid prices from Polymarket CLOB API.
///
/// Returns (yes_price, no_price) — both sampled via `/midpoint`, which is
/// the SAME semantic the backtest scraper reads from `/prices-history`:
/// the midpoint between best bid and best ask. Using this endpoint (rather
/// than `/price?side=buy`, which returns the best bid and under-reports by
/// half-spread) keeps live drift values comparable to the BT distribution
/// the strategy was calibrated on.
///
/// Diagnostic logs: if the sum (yes_mid + no_mid) drifts more than 3% from
/// 1.0, we log a warning — that's an early warning that either the book is
/// exceptionally wide (low liquidity) or the midpoint endpoint returned
/// stale data. Either case is useful to flag rather than silently bet on.
///
/// Returns (0.0, 0.0) on hard error.
async fn fetch_token_prices(yes_token_id: &str, no_token_id: &str) -> (f64, f64) {
    let _permit = clob_semaphore().acquire_owned().await;
    let client = reqwest::Client::new();

    async fn fetch_midpoint(client: &reqwest::Client, token_id: &str, label: &str) -> f64 {
        // Primary: /midpoint — fair mid between best bid and best ask.
        let url = format!("https://clob.polymarket.com/midpoint?token_id={}", token_id);
        match client.get(&url).timeout(std::time::Duration::from_secs(5)).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(v) = resp.json::<serde_json::Value>().await {
                    if let Some(p) = v.get("mid").and_then(|x| x.as_str())
                        .and_then(|s| s.parse::<f64>().ok())
                    {
                        return p;
                    }
                    // Some deployments key by "midpoint" instead of "mid"
                    if let Some(p) = v.get("midpoint").and_then(|x| x.as_str())
                        .and_then(|s| s.parse::<f64>().ok())
                    {
                        return p;
                    }
                }
                tracing::warn!("[PRICE] {} /midpoint returned unparseable body for {}", label, token_id);
            }
            Ok(resp) => {
                tracing::warn!("[PRICE] {} /midpoint failed: {} for token {}", label, resp.status(), token_id);
            }
            Err(e) => {
                tracing::warn!("[PRICE] {} /midpoint request error: {} for token {}", label, e, token_id);
            }
        }

        // Fallback: /price?side=buy returns the best bid — under-reports by
        // half-spread vs BT, but better than 0 (script stays flat if 0).
        let url = format!("https://clob.polymarket.com/price?token_id={}&side=buy", token_id);
        match client.get(&url).timeout(std::time::Duration::from_secs(5)).send().await {
            Ok(resp) if resp.status().is_success() => resp.json::<serde_json::Value>().await
                .ok()
                .and_then(|v| v.get("price").and_then(|p| p.as_str()).map(|s| s.to_string()))
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.0),
            _ => 0.0,
        }
    }

    let yes_price = fetch_midpoint(&client, yes_token_id, "YES").await;
    let no_price = fetch_midpoint(&client, no_token_id, "NO").await;

    // Sanity check: yes_mid + no_mid should be ~= 1.0 for a well-priced
    // binary market. >3% deviation suggests wide spread or stale price —
    // surface it so the operator can correlate with losing streaks.
    if yes_price > 0.0 && no_price > 0.0 {
        let sum = yes_price + no_price;
        if (sum - 1.0).abs() > 0.03 {
            tracing::warn!(
                "[PRICE] yes+no sum deviates from 1.0: yes={:.4} no={:.4} sum={:.4} (yes_tok={} no_tok={})",
                yes_price, no_price, sum, yes_token_id, no_token_id
            );
        }
    }

    (yes_price, no_price)
}

// ── Polymarket Data API trade cache (for backtesting calibration) ────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CachedTrade {
    timestamp: i64,
    price: f64,
    size: f64,
    side: String,
    outcome: String,
}

async fn load_trade_cache(condition_id: &str, workspace_dir: &std::path::Path) -> Vec<CachedTrade> {
    let file = workspace_dir
        .join("polymarket_trade_cache")
        .join(format!("{condition_id}.json"));
    if !file.exists() {
        return Vec::new();
    }
    match tokio::fs::read_to_string(&file).await {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(e) => {
            tracing::warn!("[TRADE-CACHE] Failed to read cache for {}: {}", condition_id, e);
            Vec::new()
        }
    }
}

async fn save_trade_cache(condition_id: &str, workspace_dir: &std::path::Path, trades: &[CachedTrade]) {
    let dir = workspace_dir.join("polymarket_trade_cache");
    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
        tracing::warn!("[TRADE-CACHE] Failed to create cache dir: {}", e);
        return;
    }
    let file = dir.join(format!("{condition_id}.json"));
    match serde_json::to_string_pretty(trades) {
        Ok(json) => {
            if let Err(e) = tokio::fs::write(&file, json).await {
                tracing::warn!("[TRADE-CACHE] Failed to write cache for {}: {}", condition_id, e);
            }
        }
        Err(e) => tracing::warn!("[TRADE-CACHE] Failed to serialize cache for {}: {}", condition_id, e),
    }
}

#[derive(serde::Deserialize)]
struct DataApiTradeItem {
    price: f64,
    #[serde(rename = "size")]
    size: f64,
    #[serde(rename = "timestamp")]
    ts: i64,
    side: String,
    #[serde(default)]
    outcome: String,
}

/// Fetch trades from Polymarket Data API and merge with local cache.
async fn update_trade_cache(condition_id: &str, workspace_dir: &std::path::Path) -> Vec<CachedTrade> {
    let mut cached = load_trade_cache(condition_id, workspace_dir).await;
    let client = reqwest::Client::new();
    let url = format!(
        "https://data-api.polymarket.com/trades?market={}&limit=10000",
        condition_id
    );
    match client.get(&url).timeout(std::time::Duration::from_secs(15)).send().await {
        Ok(resp) if resp.status().is_success() => {
            let body = resp.text().await.unwrap_or_default();
            match serde_json::from_str::<Vec<DataApiTradeItem>>(&body) {
                Ok(items) => {
                    let mut new_count = 0;
                    for item in items {
                        let price = item.price;
                        let size = item.size;
                        if price <= 0.0 || size <= 0.0 {
                            continue;
                        }
                        // Simple dedup: same timestamp + price + size + side
                        let exists = cached.iter().any(|c| {
                            c.timestamp == item.ts
                                && (c.price - price).abs() < 0.0001
                                && (c.size - size).abs() < 0.0001
                                && c.side == item.side
                        });
                        if !exists {
                            cached.push(CachedTrade {
                                timestamp: item.ts,
                                price,
                                size,
                                side: item.side,
                                outcome: item.outcome,
                            });
                            new_count += 1;
                        }
                    }
                    cached.sort_by_key(|c| c.timestamp);
                    tracing::info!(
                        "[TRADE-CACHE] {}: {} total trades ({} new)",
                        condition_id, cached.len(), new_count
                    );
                }
                Err(e) => {
                    let preview = if body.len() > 500 { &body[..500] } else { &body };
                    tracing::warn!(
                        "[TRADE-CACHE] Failed to parse trades for {}: {} | body preview: {}",
                        condition_id, e, preview
                    );
                }
            }
        }
        Ok(resp) => {
            let status = resp.status();
            let body_preview = resp.text().await.unwrap_or_default();
            let preview = if body_preview.len() > 200 { &body_preview[..200] } else { &body_preview };
            tracing::warn!(
                "[TRADE-CACHE] Data API returned {} for {} | body: {}",
                status, condition_id, preview
            );
        }
        Err(e) => {
            tracing::warn!("[TRADE-CACHE] Data API request failed for {}: {}", condition_id, e);
        }
    }
    save_trade_cache(condition_id, workspace_dir, &cached).await;
    cached
}

/// Determine the outcome of a completed window from the candle buffer.
/// Returns Some(true) for YES/UP, Some(false) for NO/DOWN, or None if
/// insufficient data.
fn resolve_window_outcome(
    buffer: &std::collections::VecDeque<crate::tools::backtest::Candle>,
    window_start: i64,
    window_secs: i64,
    resolution_logic: &str,
    threshold: Option<f64>,
) -> Option<bool> {
    let window_start_ms = window_start * 1000;
    let window_end_ms = (window_start + window_secs) * 1000;

    let first = buffer.iter().find(|c| c.open_time_ms >= window_start_ms)?;
    let last = buffer.iter().rev().find(|c| c.open_time_ms < window_end_ms)?;

    let went_up = match resolution_logic {
        "threshold_above" => last.close > threshold.unwrap_or(f64::MAX),
        "threshold_below" => last.close < threshold.unwrap_or(f64::MIN),
        _ => last.close > first.open,
    };

    Some(went_up)
}

/// Patch `[polymarket]` api_key/secret/passphrase in config.toml after renewal.
/// Uses line-based replacement (no full TOML parse) so it tolerates the slight
/// schema quirks that trip up `toml::Value::parse` on some real-world configs
/// and preserves comments / formatting / ordering.
async fn persist_polymarket_creds(
    config_path: &std::path::Path,
    creds: &polymarket_trader::auth::PolyCredentials,
) -> anyhow::Result<()> {
    let raw = tokio::fs::read_to_string(config_path).await?;
    let mut out: Vec<String> = Vec::with_capacity(raw.lines().count() + 4);
    let mut in_polymarket = false;
    let mut found_section = false;
    let mut wrote_api_key = false;
    let mut wrote_secret = false;
    let mut wrote_passphrase = false;

    let api_line = format!("api_key = \"{}\"", creds.api_key);
    let secret_line = format!("secret = \"{}\"", creds.secret);
    let passphrase_line = format!("passphrase = \"{}\"", creds.passphrase);

    for line in raw.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            // Section boundary: if leaving [polymarket], append any creds we never saw.
            if in_polymarket {
                if !wrote_api_key { out.push(api_line.clone()); wrote_api_key = true; }
                if !wrote_secret { out.push(secret_line.clone()); wrote_secret = true; }
                if !wrote_passphrase { out.push(passphrase_line.clone()); wrote_passphrase = true; }
            }
            in_polymarket = trimmed.starts_with("[polymarket]");
            if in_polymarket { found_section = true; }
            out.push(line.to_string());
            continue;
        }
        if in_polymarket {
            if trimmed.starts_with("api_key") && trimmed.contains('=') {
                out.push(api_line.clone());
                wrote_api_key = true;
                continue;
            }
            if trimmed.starts_with("secret") && trimmed.contains('=') {
                out.push(secret_line.clone());
                wrote_secret = true;
                continue;
            }
            if trimmed.starts_with("passphrase") && trimmed.contains('=') {
                out.push(passphrase_line.clone());
                wrote_passphrase = true;
                continue;
            }
        }
        out.push(line.to_string());
    }

    // EOF reached while still in [polymarket] — flush any missing keys.
    if in_polymarket {
        if !wrote_api_key { out.push(api_line); }
        if !wrote_secret { out.push(secret_line); }
        if !wrote_passphrase { out.push(passphrase_line); }
    }

    if !found_section {
        anyhow::bail!("[polymarket] section not found in {}", config_path.display());
    }

    let mut updated = out.join("\n");
    if raw.ends_with('\n') && !updated.ends_with('\n') {
        updated.push('\n');
    }
    tokio::fs::write(config_path, updated).await?;
    Ok(())
}

fn append_runner_log(store: &Arc<StrategyRunnerStore>, id: &str, msg: &str) {
    let mut map = store.runners.lock().unwrap();
    if let Some(r) = map.get_mut(id) {
        // Append to error field as a log (reuse for simplicity; shown in UI)
        let existing = r.status.error.take().unwrap_or_default();
        let updated = if existing.is_empty() {
            msg.to_string()
        } else {
            format!("{existing}\n{msg}")
        };
        // Keep last 1000 log lines so the frontend scroll can show full history.
        let lines: Vec<&str> = updated.lines().collect();
        let truncated = lines.iter().rev().take(1000).rev().cloned().collect::<Vec<_>>().join("\n");
        r.status.error = Some(truncated);
    }
}

fn eval_polymarket(
    script_content: &str,
    buffer: &std::collections::VecDeque<crate::tools::backtest::Candle>,
    window_minutes: usize,
    config: &RunnerConfig,
) -> crate::tools::backtest::BacktestMetrics {
    let resolution_logic = config
        .resolution_logic
        .as_deref()
        .unwrap_or("price_up");

    crate::tools::backtest::run_polymarket_binary_on_candle_buffer(
        script_content,
        buffer.iter().cloned().collect(),
        window_minutes,
        config.initial_balance,
        config.fee_pct,
        resolution_logic,
        config.threshold,
        None,
    )
}

// ── Store helpers ─────────────────────────────────────────────────────────────

pub fn set_runner_error(store: &Arc<StrategyRunnerStore>, id: &str, msg: &str) {
    tracing::error!("[RUNNER {id}] Error: {msg}");
    let mut map = store.runners.lock().unwrap();
    if let Some(r) = map.get_mut(id) {
        r.status.status = "error".to_string();
        r.status.error  = Some(msg.to_string());
    }
    drop(map);
    store.persist();
}

pub fn set_runner_status(store: &Arc<StrategyRunnerStore>, id: &str, status: &str) {
    let mut map = store.runners.lock().unwrap();
    if let Some(r) = map.get_mut(id) {
        r.status.status = status.to_string();
    }
    drop(map);
    store.persist();
}

/// Public wrapper for `reconcile_untracked_onchain` called from the gateway.
pub async fn reconcile_untracked_onchain_pub(
    store: &Arc<StrategyRunnerStore>,
    id: &str,
    wallet: &str,
) {
    reconcile_untracked_onchain(store, id, wallet).await;
}

/// Fetch USDC/pUSD trading balance from Polymarket CLOB API + Polygon RPC.
/// Combines both sources because the CLOB API may not report pUSD balance
/// (Polymarket migrated from USDC.e to pUSD). Uses the higher of the two.
async fn fetch_usdc_balance_clob(client: &polymarket_trader::orders::ClobClient) -> Option<f64> {
    let _permit = clob_semaphore().acquire_owned().await;
    let api_bal = match client.get_api_balance().await {
        Ok(b) => {
            tracing::info!("Polymarket CLOB API balance: ${:.2}", b);
            Some(b)
        }
        Err(e) => {
            tracing::warn!("Failed to fetch Polymarket CLOB API balance: {e}");
            None
        }
    };

    let rpc_bal = match client.get_balance().await {
        Ok(b) => {
            tracing::info!("Polymarket Polygon RPC balance: ${:.2}", b);
            Some(b)
        }
        Err(e) => {
            tracing::warn!("Failed to fetch Polymarket Polygon RPC balance: {e}");
            None
        }
    };

    match (api_bal, rpc_bal) {
        (Some(a), Some(r)) => {
            let best = a.max(r);
            tracing::info!("Polymarket combined balance (CLOB ${:.2} vs RPC ${:.2}): ${:.2}", a, r, best);
            Some(best)
        }
        (Some(a), None) => Some(a),
        (None, Some(r)) => Some(r),
        (None, None) => None,
    }
}

async fn update_runner_result(
    store: &Arc<StrategyRunnerStore>,
    id: &str,
    config: &RunnerConfig,
    metrics: &crate::tools::backtest::BacktestMetrics,
    live_feed: Option<LiveFeedData>,
    wallet_address: Option<String>,
    wallet_balance: Option<f64>,
    live_orders: Option<Vec<LiveOrder>>,
    live_wins: Option<u32>,
    live_total_trades: Option<u32>,
    override_last_signal: Option<String>,
) {
    let last_signal = override_last_signal
        .or_else(|| metrics.all_trades.last().map(|t| t.side.clone()))
        .unwrap_or_else(|| "flat".to_string());

    // Preserve existing live counters/orders + kv_state if not explicitly
    // provided. Defensive guard: if a caller passes an EMPTY vec (e.g. a
    // freshly-allocated `Vec::new()` in a runner-loop init path) but the
    // store already has a non-empty trade history, prefer the existing one.
    // Without this guard, an init-time `update_runner_result(.., Some(vec![]), ..)`
    // would silently wipe the dry-run / live history accumulated across
    // pause/restart cycles.
    let (orders, wins, total, kv_state) = {
        let map = store.runners.lock().unwrap();
        if let Some(ref existing) = map.get(id).and_then(|r| r.result.as_ref()) {
            let merged_orders = match live_orders {
                Some(v) if v.is_empty() && !existing.live_orders.is_empty() => {
                    existing.live_orders.clone()
                }
                Some(v) => v,
                None => existing.live_orders.clone(),
            };
            let merged_wins = match live_wins {
                Some(0) if existing.live_wins > 0 => existing.live_wins,
                Some(v) => v,
                None => existing.live_wins,
            };
            let merged_total = match live_total_trades {
                Some(0) if existing.live_total_trades > 0 => existing.live_total_trades,
                Some(v) => v,
                None => existing.live_total_trades,
            };
            (
                merged_orders,
                merged_wins,
                merged_total,
                existing.live_kv_state.clone(),
            )
        } else {
            (
                live_orders.unwrap_or_default(),
                live_wins.unwrap_or(0),
                live_total_trades.unwrap_or(0),
                std::collections::HashMap::new(),
            )
        }
    };

    // For live/paper polymarket runners, compute total_return and balance
    // from the ACTUAL per-order P&L history — NOT the rolling backtest
    // metrics, which are a separate offline simulation.  This is critical
    // when multiple runners share the same wallet: the wallet balance is
    // shared, but each runner's P&L must be isolated to its own trade log.
    let (live_return_pct, live_balance, live_wr) = {
        let settled_pnl: f64 = orders.iter().filter_map(|o| o.pnl).sum();
        let initial = config.initial_balance.max(1.0);
        let ret_pct = (settled_pnl / initial) * 100.0;
        let bal = initial + settled_pnl;
        let wr = if total > 0 {
            (wins as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        (ret_pct, bal, wr)
    };

    // Prefer live P&L when orders exist; fallback to backtest metrics for
    // non-polymarket or runners without any trade history yet.
    let (final_return_pct, final_balance, final_wr) = if total > 0 {
        (live_return_pct, live_balance, live_wr)
    } else {
        (
            metrics.total_return_pct,
            config.initial_balance * (1.0 + metrics.total_return_pct / 100.0),
            metrics.win_rate_pct,
        )
    };

    let result = RunnerResult {
        total_return_pct: final_return_pct,
        balance: final_balance,
        position: metrics.position,
        total_trades: total,
        win_rate_pct: final_wr,
        sharpe_ratio: metrics.sharpe_ratio,
        max_drawdown_pct: metrics.max_drawdown_pct,
        all_trades: metrics.all_trades.clone(),
        last_signal,
        analysis: metrics.analysis.clone(),
        live_feed,
        wallet_address,
        wallet_balance_usdc: wallet_balance,
        live_orders: orders,
        live_wins: wins,
        live_total_trades: total,
        live_kv_state: kv_state,
    };
    let mut map = store.runners.lock().unwrap();
    if let Some(r) = map.get_mut(id) {
        r.result = Some(result);
        r.status.status = "running".to_string();
    }
}

// ── Utility ───────────────────────────────────────────────────────────────────

/// Extracts the Binance trading pair from a RunnerConfig for polymarket_binary.
/// The config.symbol may be a Polymarket slug (btc-updown-5m-...) or already
/// a valid Binance pair (BTCUSDT). Returns the correct Binance symbol either way.
fn binance_symbol_for_polymarket(symbol: &str) -> String {
    let s = symbol.to_uppercase();
    // Already a valid Binance pair
    if s.ends_with("USDT") || s.ends_with("BTC") || s.ends_with("ETH") {
        return s;
    }
    // Extract from Polymarket slug (e.g. "sol-updown-5m-..." → SOLUSDT)
    let lower = symbol.to_lowercase();
    if lower.starts_with("btc") || lower.contains("-btc-") { return "BTCUSDT".to_string(); }
    if lower.starts_with("eth") || lower.contains("-eth-") { return "ETHUSDT".to_string(); }
    if lower.starts_with("sol") || lower.contains("-sol-") { return "SOLUSDT".to_string(); }
    if lower.starts_with("xrp") || lower.contains("-xrp-") { return "XRPUSDT".to_string(); }
    if lower.starts_with("doge") || lower.contains("-doge-") { return "DOGEUSDT".to_string(); }
    if lower.starts_with("hype") || lower.contains("-hype-") { return "HYPEUSDT".to_string(); }
    if lower.starts_with("bnb") || lower.contains("-bnb-") { return "BNBUSDT".to_string(); }
    // Weather / other series: no Binance symbol; return as-is
    s
}

/// Format script debug values (score, win_pct, mom1, ema14, rsi, etc.) into a single string.
fn format_debug_values(debug: &std::collections::HashMap<String, f64>) -> String {
    let ordered_keys = [
        "debug_score", "debug_win_pct", "debug_mom1", "debug_mom5",
        "debug_ema9", "debug_ema21", "debug_ema14", "debug_sma50",
        "debug_rsi", "debug_token_price", "debug_min_score",
        "debug_avg_vol", "debug_volume",
        "debug_est_prob", "debug_implied_p", "debug_edge",
    ];
    let mut parts: Vec<String> = Vec::new();
    for key in &ordered_keys {
        if let Some(v) = debug.get(*key) {
            let short = key.strip_prefix("debug_").unwrap_or(key);
            parts.push(format!("{}={:.4}", short, v));
        }
    }
    for (key, v) in debug {
        if !key.starts_with("debug_") || key == "debug_reason" {
            continue;
        }
        if ordered_keys.contains(&key.as_str()) {
            continue;
        }
        let short = key.strip_prefix("debug_").unwrap_or(key);
        parts.push(format!("{}={:.4}", short, v));
    }
    parts.join(" | ")
}

fn interval_to_secs(interval: &str) -> u64 {
    let s = interval.trim().to_lowercase();
    if s.ends_with('s') {
        s.trim_end_matches('s').parse::<u64>().unwrap_or(60)
    } else if s.ends_with('m') {
        s.trim_end_matches('m').parse::<u64>().unwrap_or(1) * 60
    } else if s.ends_with('h') {
        s.trim_end_matches('h').parse::<u64>().unwrap_or(1) * 3600
    } else if s.ends_with('d') {
        s.trim_end_matches('d').parse::<u64>().unwrap_or(1) * 86400
    } else {
        s.parse::<u64>().unwrap_or(60) * 60
    }
}

/// Compute BTC 1-hour realized volatility from the candle buffer.
/// Uses the last 60 1-minute close prices (stdev of log-returns).
/// Returns None if fewer than 5 closes are available.
fn compute_btc_rv_1h(buffer: &std::collections::VecDeque<crate::tools::backtest::Candle>) -> Option<f64> {
    let closes: Vec<f64> = buffer.iter().rev().take(61).map(|c| c.close).collect();
    if closes.len() < 5 {
        return None;
    }
    let returns: Vec<f64> = closes.windows(2)
        .filter(|w| w[0] > 0.0 && w[1] > 0.0)
        .map(|w| (w[0] / w[1]).ln())
        .collect();
    if returns.len() < 4 {
        return None;
    }
    let mean = returns.iter().sum::<f64>() / returns.len() as f64;
    let var = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (returns.len() - 1) as f64;
    Some(var.sqrt())
}

// ── CLOB 1 Hz live runner (rhai_tick) ────────────────────────────────────────
//
// Mirrors `run_clob_1hz_backtest` but drives `on_tick(ctx)` against the live
// Polymarket CLOB book once per second. Resolves windows automatically when
// `window_secs_left == 0`, places paper or live orders via `bet_yes/bet_no`,
// and persists results into the runner store every second.

#[derive(Default)]
struct TickPendingOrder {
    side: String, // "yes" or "no"
    stake_usdc: f64,
    entry_price: f64,
}

struct TickRunnerState {
    balance: f64,
    position: i64,           // 0 = flat, 1 = YES, -1 = NO
    entry_price: f64,        // ask price paid per share (paper accounting)
    stake: f64,              // paper stake currently at risk
    window_open_price: f64,  // binance_price at start of current window
    current_window_ts: i64,
    kv: std::collections::HashMap<String, f64>,
    pending: Option<TickPendingOrder>,
}

async fn tick_runner_loop(
    store: Arc<StrategyRunnerStore>,
    mut config: RunnerConfig,
    workspace_dir: PathBuf,
) {
    use rhai::{Dynamic, Engine, Map, Scope};

    let id = config.id.clone();
    let is_live = config.mode == "live";
    let window_secs: i64 = interval_to_secs(&config.interval).max(60) as i64;

    // 1) Resolve script
    let script_content = match crate::tools::backtest::read_script_or_default(&workspace_dir, &config.script) {
        Some(s) => s,
        None => {
            set_runner_error(&store, &id, &format!("Script not found: {}", config.script));
            return;
        }
    };

    // 2) Validate & pre-compile Rhai AST (must define on_tick)
    let engine_check = Engine::new();
    let ast_check = match engine_check.compile(&script_content) {
        Ok(a) => a,
        Err(e) => {
            set_runner_error(&store, &id, &format!("Script compile error: {e}"));
            return;
        }
    };
    if !ast_check.iter_functions().any(|f| f.name == "on_tick") {
        set_runner_error(
            &store, &id,
            "rhai_tick engine requires an `on_tick(ctx)` function. \
             Use `ctx.bet_yes(size)` / `ctx.bet_no(size)` to place bets.",
        );
        return;
    }

    // 3) Live mode: build CLOB client (same pattern as polymarket_runner_loop)
    let mut clob_client: Option<Arc<polymarket_trader::orders::ClobClient>> = if is_live {
        if config.series_id.as_deref().unwrap_or("").trim().is_empty() {
            set_runner_error(
                &store, &id,
                "Live mode cannot start: no Market Series selected. Recreate the strategy and choose a supported series.",
            );
            return;
        }
        match &config.poly_creds {
            None => {
                set_runner_error(
                    &store, &id,
                    "Live mode requires Polymarket credentials. Set api_key, secret, and passphrase in Settings → Config → [polymarket].",
                );
                return;
            }
            Some(creds) => {
                if creds.api_key.is_empty() || creds.secret.is_empty() || creds.passphrase.is_empty() {
                    set_runner_error(
                        &store, &id,
                        "Live mode: Polymarket credentials incomplete. Check api_key, secret, and passphrase in Settings → Config.",
                    );
                    return;
                }
                Some(Arc::new(polymarket_trader::orders::ClobClient::new(creds.clone())))
            }
        }
    } else {
        None
    };

    // 4) Binance miniTicker WS for real-time price (used for window resolution)
    let binance_sym = binance_symbol_for_polymarket(&config.symbol);
    let binance_price = Arc::new(std::sync::RwLock::new(0f64));
    {
        let bp_write = binance_price.clone();
        let mut rx = crate::live_feed::spawn_binance_ticker_feed(binance_sym.clone());
        tokio::spawn(async move {
            while let Some(p) = rx.recv().await {
                *bp_write.write().unwrap() = p;
            }
        });
    }

    // 5) Build the persistent Rhai engine + shared scalars + closures
    let mut eng = Engine::new();
    eng.set_max_operations(200_000);
    eng.set_max_call_levels(32);

    let patched = script_content
        .replace("ctx.bet_yes(", "bet_yes_impl(")
        .replace("ctx.bet_no(",  "bet_no_impl(")
        .replace("ctx.set(",     "set_impl(")
        .replace("ctx.get(",     "get_impl(");

    let ast = match eng.compile(&patched) {
        Ok(a) => a,
        Err(e) => {
            set_runner_error(&store, &id, &format!("Script compile error: {e}"));
            return;
        }
    };

    // Restore prior kv state if a runner was resumed
    let restored_kv: std::collections::HashMap<String, f64> = {
        let map = store.runners.lock().unwrap();
        map.get(&id)
            .and_then(|r| r.result.as_ref())
            .map(|res| res.live_kv_state.clone())
            .unwrap_or_default()
    };
    let restored_balance: f64 = {
        let map = store.runners.lock().unwrap();
        map.get(&id)
            .and_then(|r| r.result.as_ref())
            .map(|res| res.balance)
            .unwrap_or(config.initial_balance)
    };

    let state = Arc::new(Mutex::new(TickRunnerState {
        balance: if restored_balance > 0.0 { restored_balance } else { config.initial_balance },
        position: 0,
        entry_price: 0.0,
        stake: 0.0,
        window_open_price: 0.0,
        current_window_ts: 0,
        kv: restored_kv,
        pending: None,
    }));

    // Shared per-tick scalars (yes_ask, no_ask, balance, fee, max_pos + book depth)
    let cur_yes_ask   = Arc::new(Mutex::new(0f64));
    let cur_no_ask    = Arc::new(Mutex::new(0f64));
    let cur_yes_bid   = Arc::new(Mutex::new(0f64));
    let cur_balance   = Arc::new(Mutex::new(0f64));
    let cur_fee       = Arc::new(Mutex::new(config.fee_pct));
    let cur_ask_depth = Arc::new(Mutex::new(0f64));
    let cur_bid_depth = Arc::new(Mutex::new(0f64));
    let cap_per_bet: f64 = match config.live_sizing_mode {
        LiveSizingMode::Fixed => config.live_sizing_value.max(5.0),
        LiveSizingMode::Percent => {
            // live_sizing_value stored as 0–100 percent of initial_balance
            let frac = (config.live_sizing_value / 100.0).clamp(0.0, 1.0);
            (config.initial_balance * frac).max(5.0)
        }
    };
    let max_pos_usd_arc = Arc::new(Mutex::new(cap_per_bet));

    // bet_yes_impl
    {
        let st = state.clone();
        let yes_ask_ref = cur_yes_ask.clone();
        let yes_bid_ref = cur_yes_bid.clone();
        let ask_depth_ref = cur_ask_depth.clone();
        let bal_ref = cur_balance.clone();
        let fee_ref = cur_fee.clone();
        let max_pos_ref = max_pos_usd_arc.clone();
        eng.register_fn("bet_yes_impl", move |size: f64| {
            let ya    = *yes_ask_ref.lock().unwrap();
            let yb    = *yes_bid_ref.lock().unwrap();
            let depth = *ask_depth_ref.lock().unwrap();
            let bal = *bal_ref.lock().unwrap();
            let f   = *fee_ref.lock().unwrap();
            let max_pos = *max_pos_ref.lock().unwrap();
            let mut s = st.lock().unwrap();
            if s.position != 0 { return; }
            if ya < 0.03 || ya >= 1.0 { return; }
            let frac = size.clamp(0.0, 1.0);
            let stake_amt = (bal * frac * (1.0 - f / 100.0)).min(max_pos);
            if stake_amt <= 0.01 { return; }
            // Paper fill = book-VWAP (live-parity with on_tick backtester).
            // For live mode the real fill replaces this via reconcile after placement.
            let fill_price = crate::tools::backtest::sim_fill_vwap(ya, yb, depth, stake_amt);
            // Reserve immediately for paper accounting; live placement happens
            // after on_tick returns. Position flag is set so subsequent ticks
            // in this window cannot place a second bet.
            s.balance -= stake_amt;
            s.position = 1;
            s.entry_price = fill_price;
            s.stake = stake_amt;
            s.pending = Some(TickPendingOrder {
                side: "yes".to_string(),
                stake_usdc: stake_amt,
                entry_price: fill_price,
            });
        });
    }

    // bet_no_impl
    {
        let st = state.clone();
        let no_ask_ref = cur_no_ask.clone();
        let yes_ask_ref2 = cur_yes_ask.clone(); // yes_ask → no_bid = 1 - yes_ask
        let bid_depth_ref = cur_bid_depth.clone();
        let bal_ref = cur_balance.clone();
        let fee_ref = cur_fee.clone();
        let max_pos_ref = max_pos_usd_arc.clone();
        eng.register_fn("bet_no_impl", move |size: f64| {
            let na    = *no_ask_ref.lock().unwrap();
            let no_bid = (1.0 - *yes_ask_ref2.lock().unwrap()).clamp(0.0, 1.0);
            let depth = *bid_depth_ref.lock().unwrap();
            let bal = *bal_ref.lock().unwrap();
            let f   = *fee_ref.lock().unwrap();
            let max_pos = *max_pos_ref.lock().unwrap();
            let mut s = st.lock().unwrap();
            if s.position != 0 { return; }
            if na < 0.03 || na >= 1.0 { return; }
            let frac = size.clamp(0.0, 1.0);
            let stake_amt = (bal * frac * (1.0 - f / 100.0)).min(max_pos);
            if stake_amt <= 0.01 { return; }
            let fill_price = crate::tools::backtest::sim_fill_vwap(na, no_bid, depth, stake_amt);
            s.balance -= stake_amt;
            s.position = -1;
            s.entry_price = fill_price;
            s.stake = stake_amt;
            s.pending = Some(TickPendingOrder {
                side: "no".to_string(),
                stake_usdc: stake_amt,
                entry_price: fill_price,
            });
        });
    }

    // set_impl / get_impl
    {
        let st = state.clone();
        eng.register_fn("set_impl", move |key: String, val: rhai::Dynamic| {
            let f = if val.is::<f64>() { val.clone().as_float().ok() }
                    else if val.is::<i64>() { val.clone().as_int().ok().map(|i| i as f64) }
                    else if val.is::<bool>() { val.clone().as_bool().ok().map(|b| if b { 1.0 } else { 0.0 }) }
                    else { None };
            if let Some(v) = f {
                st.lock().unwrap().kv.insert(key, v);
            }
        });
    }
    {
        let st = state.clone();
        eng.register_fn("get_impl", move |key: String, default: rhai::Dynamic| -> f64 {
            let def = if default.is::<f64>() { default.clone().as_float().ok().unwrap_or(0.0) }
                      else if default.is::<i64>() { default.clone().as_int().ok().map(|i| i as f64).unwrap_or(0.0) }
                      else if default.is::<bool>() { default.clone().as_bool().ok().map(|b| if b { 1.0 } else { 0.0 }).unwrap_or(0.0) }
                      else { 0.0 };
            st.lock().unwrap().kv.get(&key).copied().unwrap_or(def)
        });
    }

    // 6) HTTP client for /book polling
    let http = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            set_runner_error(&store, &id, &format!("HTTP client init failed: {e}"));
            return;
        }
    };

    set_runner_status(&store, &id, "running");
    tracing::info!(
        "[RUNNER {id}] rhai_tick engine started — script={} symbol={} cap=${:.0} mode={}",
        config.script, binance_sym, cap_per_bet, config.mode
    );
    append_runner_log(
        &store, &id,
        &format!(
            "rhai_tick engine started: script={} symbol={} cap=${:.0} mode={}",
            config.script, binance_sym, cap_per_bet, config.mode
        ),
    );

    // 7) 1-Hz tick loop
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));

    let mut last_window: i64 = -1;
    let mut live_orders: Vec<LiveOrder> = {
        let map = store.runners.lock().unwrap();
        map.get(&id)
            .and_then(|r| r.result.as_ref())
            .map(|res| res.live_orders.clone())
            .unwrap_or_default()
    };
    let mut live_wins: u32 = {
        let map = store.runners.lock().unwrap();
        map.get(&id).and_then(|r| r.result.as_ref()).map(|res| res.live_wins).unwrap_or(0)
    };
    let mut live_total: u32 = {
        let map = store.runners.lock().unwrap();
        map.get(&id).and_then(|r| r.result.as_ref()).map(|res| res.live_total_trades).unwrap_or(0)
    };

    loop {
        interval.tick().await;

        // Stop if the runner was deleted from the store
        let still_present = store.runners.lock().unwrap().contains_key(&id);
        if !still_present { break; }

        let now_s = chrono::Utc::now().timestamp();
        let rem = now_s % window_secs;
        let (window_ts, window_secs_left) = if rem == 0 {
            (now_s - window_secs, 0)
        } else {
            (now_s - rem, window_secs - rem)
        };
        let second_in_window = window_secs - window_secs_left;

        // ── New window: resolve token ids for this window's market slug ───
        if window_ts != last_window {
            if let Some(series) = config.series_id.as_deref().filter(|s| !s.is_empty()) {
                match resolve_token_for_window(series, window_ts as u64).await {
                    Ok((yes_tid, no_tid, cond_id)) => {
                        config.poly_token_id = Some(yes_tid);
                        config.poly_no_token_id = Some(no_tid);
                        config.poly_condition_id = Some(cond_id);
                    }
                    Err(e) => {
                        tracing::warn!("[RUNNER {id}] Token resolve failed for window {window_ts}: {e}");
                    }
                }
            }
            last_window = window_ts;
        }

        // ── Fetch CLOB book (yes_bid/yes_ask/no_bid/no_ask + depth) ───────
        let yes_token = config.poly_token_id.clone().unwrap_or_default();
        let (yes_bid, yes_ask, no_bid, no_ask, ask_depth_usd, bid_depth_usd) = if !yes_token.is_empty() {
            fetch_clob_book(&http, &yes_token).await
        } else {
            (0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        };
        let yes_mid = if yes_bid > 0.0 && yes_ask > 0.0 { (yes_bid + yes_ask) / 2.0 }
                      else if yes_bid > 0.0 { yes_bid }
                      else { yes_ask };
        let spread_pct = if yes_bid > 0.0 && yes_ask > 0.0 { (yes_ask - yes_bid) * 100.0 } else { 0.0 };
        let bp = *binance_price.read().unwrap();

        // ── Window resolution: settle position when window closes ─────────
        {
            let mut s = state.lock().unwrap();
            // First-time anchor of window_open_price for the current window
            // Extract all state we need before any potential await, then drop the guard.
            let need_advance = s.current_window_ts == 0 || window_ts != s.current_window_ts;
            let prev_wts    = s.current_window_ts;
            let stake       = s.stake;
            let entry_price = s.entry_price;
            let position    = s.position;
            let wopen       = s.window_open_price;
            let has_pos     = position != 0 && wopen > 0.0 && bp > 0.0;
            drop(s); // always drop before any async work

            if need_advance {
                // Window changed → settle previous open position.
                // Strategy: settle immediately with Binance price (synchronous, no await),
                // then ALWAYS spawn a background monitor that retries the official Polymarket
                // oracle every 30 s.  When the oracle settles (usually 60-180 s later), the
                // monitor patches the order with the correct result and corrects the P&L.
                if has_pos {
                    let yes_won_binance = bp > wopen;
                    let won = (position == 1 && yes_won_binance) || (position == -1 && !yes_won_binance);
                    let pnl = if won && entry_price > 0.0 {
                        stake / entry_price - stake
                    } else { -stake };

                    {
                        let mut s2 = state.lock().unwrap();
                        if won { s2.balance += if entry_price > 0.0 { stake / entry_price } else { 0.0 }; }
                        s2.position = 0; s2.entry_price = 0.0; s2.stake = 0.0;
                    }

                    if let Some(order) = live_orders.iter_mut().rev().find(|o| {
                        o.window_ts == prev_wts && o.result.is_none()
                    }) {
                        order.result = Some(if won { "WIN".to_string() } else { "LOSS".to_string() });
                        order.pnl = Some(pnl);
                        order.resolution_yes_won = Some(yes_won_binance);
                        order.resolution_source = Some("binance_provisional".to_string());
                    }

                    if won { live_wins = live_wins.saturating_add(1); }
                    live_total = live_total.saturating_add(1);

                    append_runner_log(
                        &store, &id,
                        &format!(
                            "Window {} provisional: {} via Binance (open={:.2} close={:.2} pnl={:+.2}) — awaiting oracle",
                            prev_wts, if won { "WIN" } else { "LOSS" }, wopen, bp, pnl
                        ),
                    );

                    // Always spawn the official oracle monitor — it will correct the result
                    // when Polymarket/Chainlink settles (typically 60-180 s after window close).
                    if let Some(ref sid) = config.series_id {
                        if !sid.is_empty() {
                            spawn_resolution_monitor(
                                id.to_string(), sid.clone(), prev_wts, store.clone(), yes_won_binance,
                                Some(state.clone()),
                                config.fee_pct,
                            );
                        }
                    }
                }

                {
                    let mut s2 = state.lock().unwrap();
                    s2.current_window_ts = window_ts;
                    s2.window_open_price = if bp > 0.0 { bp } else { 0.0 };
                    tracing::info!(
                        "[RUNNER {id}] new window {} anchored open={:.2} (yes_token={}…)",
                        window_ts, s2.window_open_price,
                        config.poly_token_id.as_deref().unwrap_or("").chars().take(10).collect::<String>(),
                    );
                }
            } else if wopen <= 0.0 && bp > 0.0 {
                state.lock().unwrap().window_open_price = bp;
            }
        }

        // ── Build ctx_map and run on_tick ─────────────────────────────────
        let (cur_pos, cur_entry, cur_bal, cur_window_open) = {
            let s = state.lock().unwrap();
            (s.position, s.entry_price, s.balance, s.window_open_price)
        };
        *cur_yes_ask.lock().unwrap()   = yes_ask;
        *cur_no_ask.lock().unwrap()    = no_ask;
        *cur_yes_bid.lock().unwrap()   = yes_bid;
        *cur_balance.lock().unwrap()   = cur_bal;
        *cur_ask_depth.lock().unwrap() = ask_depth_usd;
        *cur_bid_depth.lock().unwrap() = bid_depth_usd;

        let mut ctx_map = Map::new();
        ctx_map.insert("ts_ms".into(),            Dynamic::from(now_s * 1000));
        ctx_map.insert("yes_bid".into(),          Dynamic::from(yes_bid));
        ctx_map.insert("yes_ask".into(),          Dynamic::from(yes_ask));
        ctx_map.insert("yes_mid".into(),          Dynamic::from(yes_mid));
        ctx_map.insert("no_bid".into(),           Dynamic::from(no_bid));
        ctx_map.insert("no_ask".into(),           Dynamic::from(no_ask));
        ctx_map.insert("spread_pct".into(),       Dynamic::from(spread_pct));
        ctx_map.insert("binance_price".into(),    Dynamic::from(bp));
        ctx_map.insert("window_ts".into(),        Dynamic::from(window_ts));
        ctx_map.insert("window_secs_left".into(), Dynamic::from(window_secs_left));
        ctx_map.insert("second_in_window".into(), Dynamic::from(second_in_window));
        ctx_map.insert("balance".into(),          Dynamic::from(cur_bal));
        ctx_map.insert("position".into(),         Dynamic::from(cur_pos));
        ctx_map.insert("entry_price".into(),      Dynamic::from(cur_entry));
        ctx_map.insert("window_open_price".into(), Dynamic::from(cur_window_open));
        ctx_map.insert("ask_depth_usd".into(),    Dynamic::from(ask_depth_usd));
        ctx_map.insert("bid_depth_usd".into(),    Dynamic::from(bid_depth_usd));

        let mut scope = Scope::new();
        if let Err(e) = eng.call_fn::<Dynamic>(&mut scope, &ast, "on_tick", (ctx_map,)) {
            tracing::warn!("[RUNNER {id}] on_tick error: {e}");
        }

        // ── Diagnostic snapshot: log every 10s in entry zone ──────────────
        if window_secs_left > 0 && window_secs_left <= 60 && (window_secs_left % 10 == 0) {
            let wopen = state.lock().unwrap().window_open_price;
            let change_pct = if wopen > 0.0 && bp > 0.0 {
                (bp - wopen) / wopen * 100.0
            } else { 0.0 };
            tracing::info!(
                "[RUNNER {id}] tick secs_left={ws} bp={bp:.2} open={wopen:.2} change={change_pct:.4}% \
                 yes={yes_bid:.3}/{yes_mid:.3}/{yes_ask:.3} no_ask={no_ask:.3} spread={spread_pct:.2}c \
                 pos={cur_pos} pending={pending}",
                ws = window_secs_left,
                bp = bp, wopen = wopen, change_pct = change_pct,
                yes_bid = yes_bid, yes_mid = yes_mid, yes_ask = yes_ask, no_ask = no_ask,
                spread_pct = spread_pct, cur_pos = cur_pos,
                pending = state.lock().unwrap().pending.is_some(),
            );
        }

        // ── Process pending order (gates → place paper or live) ───────────
        let pending = state.lock().unwrap().pending.take();
        if let Some(po) = pending {
            // Gate: max_spread_pct
            let mut skip_reason: Option<String> = None;
            if let Some(max_sp) = config.max_spread_pct {
                if max_sp > 0.0 && spread_pct / 100.0 > max_sp {
                    skip_reason = Some(format!("spread {:.4} > max {:.4}", spread_pct / 100.0, max_sp));
                }
            }
            // Gate: allowed_hours (UTC)
            if skip_reason.is_none() && !config.allowed_hours.is_empty() {
                use chrono::Timelike;
                let h = chrono::Utc::now().hour() as u8;
                if !config.allowed_hours.contains(&h) {
                    skip_reason = Some(format!("hour {} not in allowed list", h));
                }
            }
            // Gate: max_entry_price
            if skip_reason.is_none() {
                if let Some(max_ep) = config.max_entry_price {
                    if po.entry_price > max_ep {
                        skip_reason = Some(format!("entry {:.4} > max {:.4}", po.entry_price, max_ep));
                    }
                }
            }
            // Gate: min_entry_price (guardrail — blocks extreme long-shot bets)
            if skip_reason.is_none() && config.min_entry_price > 0.0 {
                if po.entry_price < config.min_entry_price {
                    skip_reason = Some(format!("entry {:.4} < min {:.4}", po.entry_price, config.min_entry_price));
                }
            }

            if let Some(reason) = skip_reason {
                // Refund paper stake & clear position so next ticks can retry
                let mut s = state.lock().unwrap();
                s.balance += po.stake_usdc;
                s.position = 0;
                s.entry_price = 0.0;
                s.stake = 0.0;
                append_runner_log(&store, &id, &format!("Skipped {} bet: {}", po.side, reason));
            } else {
                let token_id = if po.side == "yes" {
                    config.poly_token_id.clone().unwrap_or_default()
                } else {
                    config.poly_no_token_id.clone().unwrap_or_default()
                };
                if token_id.is_empty() {
                    let mut s = state.lock().unwrap();
                    s.balance += po.stake_usdc;
                    s.position = 0;
                    s.entry_price = 0.0;
                    s.stake = 0.0;
                    append_runner_log(&store, &id, &format!("Skipped {} bet: token_id unresolved", po.side));
                } else {
                    let (order_opt, renewed) = execute_tick_market_order(
                        &id,
                        clob_client.clone(),
                        &po.side,
                        &token_id,
                        po.stake_usdc,
                        po.entry_price,
                        window_ts,
                        &store,
                    ).await;
                    if let Some(c) = renewed { clob_client = Some(c); }
                    if let Some(order) = order_opt {
                        // Settle on the real fill cost: paper now quotes the live CLOB
                        // (fill_price = fresh book VWAP), so the payout (stake/entry) must
                        // use that fill rather than the tick-feed price set in bet_*_impl.
                        if let Some(fp) = order.fill_price {
                            if fp > 0.0 { state.lock().unwrap().entry_price = fp; }
                        }
                        live_orders.push(order);
                        store.persist();
                    } else {
                        // Order failed: refund paper stake & flat position
                        let mut s = state.lock().unwrap();
                        s.balance += po.stake_usdc;
                        s.position = 0;
                        s.entry_price = 0.0;
                        s.stake = 0.0;
                    }
                }
            }
        }

        // ── Persist result to store every tick ────────────────────────────
        let (kv_snapshot, balance_now, position_now) = {
            let s = state.lock().unwrap();
            (s.kv.clone(), s.balance, s.position as f64)
        };
        let live_feed = LiveFeedData {
            current_btc_price: bp,
            market_slug: config.symbol.clone(),
            window_timestamp: window_ts,
            window_seconds_left: window_secs_left,
            price_to_beat: state.lock().unwrap().window_open_price,
            yes_token_price: yes_mid,
            no_token_price: if yes_bid > 0.0 && yes_ask > 0.0 {
                ((1.0 - yes_ask) + (1.0 - yes_bid)) / 2.0
            } else { 0.0 },
            price_history: vec![],
        };

        store.update_result(&id, |res| {
            res.balance = balance_now;
            res.position = position_now;
            res.live_orders = live_orders.clone();
            res.live_wins = live_wins;
            res.live_total_trades = live_total;
            res.total_trades = live_total;
            res.win_rate_pct = if live_total > 0 {
                (live_wins as f64 / live_total as f64) * 100.0
            } else { 0.0 };
            let initial = config.initial_balance.max(1.0);
            res.total_return_pct = ((balance_now - initial) / initial) * 100.0;
            res.last_signal = match position_now as i64 {
                1 => "yes".to_string(),
                -1 => "no".to_string(),
                _ => "flat".to_string(),
            };
            res.live_feed = Some(live_feed);
            res.live_kv_state = kv_snapshot;
        });
    }

    tracing::info!("[RUNNER {id}] rhai_tick loop exiting");
}

/// Fetch best bid/ask from Polymarket CLOB `/book` for a YES token id and
/// derive NO from the complement (no_bid = 1 - yes_ask, no_ask = 1 - yes_bid).
///
/// Polymarket's `/book` returns bids ordered low→high and asks low→high, so
/// best bid = max(bid prices) and best ask = min(ask prices). Many windows
/// have only one side populated (e.g. asks=0 once YES has won) — fall back to
/// the `/price` endpoint with side=BUY/SELL to fill the missing side.
/// Returns (yes_bid, yes_ask, no_bid, no_ask, ask_depth_usd, bid_depth_usd).
/// Depth = USD liquidity within 2% of the best price on each side, used by the
/// tick runner's `sim_fill_vwap` to model slippage in paper mode (live-parity
/// with the on_tick backtester).
async fn fetch_clob_book(
    client: &reqwest::Client,
    yes_token_id: &str,
) -> (f64, f64, f64, f64, f64, f64) {
    let _permit = clob_semaphore().acquire_owned().await;
    let url = format!("https://clob.polymarket.com/book?token_id={}", yes_token_id);
    let (mut yes_bid, mut yes_ask, ask_depth, bid_depth) = match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
            match resp.json::<serde_json::Value>().await {
                Ok(json) => {
                    let yb = clob_best_price(&json, "bids");
                    let ya = clob_best_price(&json, "asks");
                    let ad = clob_side_depth(&json, "asks", ya, 0.02);
                    let bd = clob_side_depth(&json, "bids", yb, 0.02);
                    (yb, ya, ad, bd)
                }
                Err(_) => (0.0, 0.0, 0.0, 0.0),
            }
        }
        _ => (0.0, 0.0, 0.0, 0.0),
    };

    // Fallback: query /price endpoint for any missing side.
    if yes_bid <= 0.0 {
        yes_bid = fetch_clob_side_price(client, yes_token_id, "SELL").await;
    }
    if yes_ask <= 0.0 {
        yes_ask = fetch_clob_side_price(client, yes_token_id, "BUY").await;
    }

    let no_bid = if yes_ask > 0.0 { (1.0 - yes_ask).max(0.0) } else { 0.0 };
    let no_ask = if yes_bid > 0.0 { (1.0 - yes_bid).max(0.0) } else { 0.0 };
    (yes_bid, yes_ask, no_bid, no_ask, ask_depth, bid_depth)
}

/// Sum USD-equivalent liquidity within `pct` of `best_price` on a book side.
/// `price × size` over all levels with price ≤ best×(1+pct) (asks) or ≥ best×(1-pct) (bids).
fn clob_side_depth(json: &serde_json::Value, side: &str, best_price: f64, pct: f64) -> f64 {
    if best_price <= 0.0 { return 0.0; }
    let arr = match json.get(side).and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return 0.0,
    };
    let (lo, hi) = if side == "asks" {
        (0.0, best_price * (1.0 + pct))
    } else {
        (best_price * (1.0 - pct), f64::INFINITY)
    };
    arr.iter().map(|entry| {
        let parse = |k1: &str, k2: &str| -> f64 {
            entry.get(k1).or_else(|| entry.get(k2))
                .and_then(|v| match v {
                    serde_json::Value::String(s) => s.parse::<f64>().ok(),
                    serde_json::Value::Number(n) => n.as_f64(),
                    _ => None,
                })
                .unwrap_or(0.0)
        };
        let price = parse("price", "p");
        let size  = parse("size", "s");
        if price >= lo && price <= hi && price > 0.0 && size > 0.0 {
            price * size
        } else { 0.0 }
    }).sum()
}

async fn fetch_clob_side_price(client: &reqwest::Client, token_id: &str, side: &str) -> f64 {
    let url = format!("https://clob.polymarket.com/price?token_id={}&side={}", token_id, side);
    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
            match resp.json::<serde_json::Value>().await {
                Ok(json) => json
                    .get("price")
                    .and_then(|v| match v {
                        serde_json::Value::String(s) => s.parse::<f64>().ok(),
                        serde_json::Value::Number(n) => n.as_f64(),
                        _ => None,
                    })
                    .unwrap_or(0.0),
                Err(_) => 0.0,
            }
        }
        _ => 0.0,
    }
}

fn clob_best_price(json: &serde_json::Value, side: &str) -> f64 {
    // Polymarket /book returns levels as arrays; bids are best=highest price,
    // asks are best=lowest price. Compute the extreme explicitly rather than
    // trusting arr.first() / arr.last() — the order is documented but has
    // changed before across CLOB versions.
    let arr = match json.get(side).and_then(|v| v.as_array()) {
        Some(a) if !a.is_empty() => a,
        _ => return 0.0,
    };
    let prices: Vec<f64> = arr.iter().filter_map(|entry| {
        let p = entry.get("price").or_else(|| entry.get("p"))?;
        match p {
            serde_json::Value::String(s) => s.parse::<f64>().ok(),
            serde_json::Value::Number(n) => n.as_f64(),
            _ => None,
        }
    }).collect();
    if prices.is_empty() {
        return 0.0;
    }
    if side == "bids" {
        prices.into_iter().fold(0f64, f64::max)
    } else {
        prices.into_iter().fold(f64::INFINITY, f64::min)
    }
}

/// Place a paper or live market order for the rhai_tick engine.
/// Returns (Some(LiveOrder), renewed_client?) on success, (None, _) on failure.
async fn execute_tick_market_order(
    id: &str,
    client: Option<Arc<polymarket_trader::orders::ClobClient>>,
    side_str: &str,            // "yes" or "no"
    token_id: &str,
    amount_usdc: f64,
    entry_price: f64,
    window_ts: i64,
    store: &Arc<StrategyRunnerStore>,
) -> (Option<LiveOrder>, Option<Arc<polymarket_trader::orders::ClobClient>>) {
    use polymarket_trader::orders::Side;

    if let Some(client_arc) = client {
        // LIVE: market order with 5% slippage cap
        let worst_price = (entry_price * 1.05).min(0.95);
        let mut renewed: Option<Arc<polymarket_trader::orders::ClobClient>> = None;
        match client_arc.create_market_order(token_id, Side::Buy, amount_usdc, worst_price).await {
            Ok(resp) => {
                append_runner_log(
                    store, id,
                    &format!("Tick order placed: {} ${:.0} @ {:.4} (id={})", side_str, amount_usdc, entry_price, resp.order_id),
                );
                (Some(LiveOrder {
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    window_ts,
                    side: side_str.to_string(),
                    token_id: token_id.to_string(),
                    amount_usdc,
                    order_id: resp.order_id,
                    status: resp.status,
                    entry_price: Some(entry_price),
                    result: None,
                    pnl: None,
                    stop_loss_triggered: false,
                    ..Default::default()
                }), renewed)
            }
            Err(e) => {
                let msg = e.to_string();
                tracing::warn!("[RUNNER {id}] Tick market order failed: {msg}");
                if msg.contains("order_version_mismatch") {
                    if let Ok(new_client) = client_arc.renew().await {
                        let new_arc = Arc::new(new_client);
                        renewed = Some(new_arc.clone());
                        if let Ok(resp) = new_arc.create_market_order(token_id, Side::Buy, amount_usdc, worst_price).await {
                            append_runner_log(
                                store, id,
                                &format!("Tick order placed after renew: {} ${:.0} @ {:.4} (id={})", side_str, amount_usdc, entry_price, resp.order_id),
                            );
                            return (Some(LiveOrder {
                                timestamp: chrono::Utc::now().to_rfc3339(),
                                window_ts,
                                side: side_str.to_string(),
                                token_id: token_id.to_string(),
                                amount_usdc,
                                order_id: resp.order_id,
                                status: resp.status,
                                entry_price: Some(entry_price),
                                result: None,
                                pnl: None,
                                stop_loss_triggered: false,
                                ..Default::default()
                            }), renewed);
                        }
                    }
                }
                append_runner_log(store, id, &format!("Tick order failed: {msg}"));
                (None, renewed)
            }
        }
    } else {
        // PAPER — quote against the live CLOB exactly like the on_candle path, instead
        // of reusing the tick-feed price: entry_price = real CLOB ask (/price?side=buy),
        // fill_price = book-walk VWAP for this stake (captures depth slippage). Falls back
        // to the feed-derived `entry_price` only when the CLOB is unreachable.
        let order_id = format!("paper-{}", chrono::Utc::now().timestamp_millis());
        let clob_ask = polymarket_trader::markets::get_market_price(token_id)
            .await
            .unwrap_or(entry_price);
        let (sim_fill, slippage_pct) = simulate_book_fill(token_id, amount_usdc, clob_ask).await;
        append_runner_log(
            store, id,
            &format!(
                "Paper tick order: {} ${:.0} clob_ask={:.4} sim_fill={:.4} slip={:.2}% (id={})",
                side_str, amount_usdc, clob_ask, sim_fill, slippage_pct, order_id
            ),
        );
        (Some(LiveOrder {
            timestamp: chrono::Utc::now().to_rfc3339(),
            window_ts,
            side: side_str.to_string(),
            token_id: token_id.to_string(),
            amount_usdc,
            order_id,
            status: "LIVE".to_string(),
            // entry_price = real CLOB ask; fill_price = book VWAP (live-parity with on_candle)
            entry_price: Some(clob_ask),
            fill_price: Some(sim_fill),
            result: None,
            pnl: None,
            stop_loss_triggered: false,
            ..Default::default()
        }), None)
    }
}

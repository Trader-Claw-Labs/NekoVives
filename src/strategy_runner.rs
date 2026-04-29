//! Live Strategy Runner — real-time paper/live trading sessions.
//!
//! Each runner:
//!  1. Fetches a warmup window of recent candles from Binance REST.
//!  2. Connects to the Binance WebSocket kline stream for real-time closed candles.
//!  3. Runs the Rhai strategy on a rolling buffer after every new closed candle.
//!  4. In paper mode: tracks simulated P&L and updates the store.
//!     In live mode (polymarket_binary only): sends real orders via Polymarket CLOB API.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use serde::{Deserialize, Serialize};
use tokio::task::AbortHandle;

// ── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LiveSizingMode {
    #[default]
    Percent,
    Fixed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveOrder {
    pub timestamp: String,
    pub window_ts: i64,
    pub side: String,
    pub token_id: String,
    pub amount_usdc: f64,
    pub order_id: String,
    pub status: String,
    pub entry_price: Option<f64>,
    pub result: Option<String>,
    pub pnl: Option<f64>,
    /// True when this position was closed early by the stop-loss monitor.
    #[serde(default)]
    pub stop_loss_triggered: bool,
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
}

pub struct StrategyRunnerStore {
    runners: Arc<Mutex<std::collections::HashMap<String, StoredRunner>>>,
    handles: Arc<Mutex<std::collections::HashMap<String, AbortHandle>>>,
    workspace_dir: PathBuf,
}

fn default_auto_restart() -> bool { true }
fn default_chainlink_interval() -> u64 { 5 }

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

    pub fn update_runner_config(
        &self,
        id: &str,
        live_sizing_mode: Option<LiveSizingMode>,
        live_sizing_value: Option<f64>,
        max_entry_price: Option<f64>,
    ) -> Option<StoredRunner> {
        let mut map = self.runners.lock().unwrap();
        let updated = map.get_mut(id).map(|r| {
            if let Some(mode) = live_sizing_mode {
                r.config.live_sizing_mode = mode;
            }
            if let Some(val) = live_sizing_value {
                r.config.live_sizing_value = val;
            }
            if let Some(val) = max_entry_price {
                r.config.max_entry_price = Some(val);
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

    pub fn delete(&self, id: &str) -> bool {
        self.stop(id);
        let removed = self.runners.lock().unwrap().remove(id).is_some();
        self.persist();
        removed
    }

    pub fn register_handle(&self, id: String, handle: AbortHandle) {
        self.handles.lock().unwrap().insert(id, handle);
    }
}

// ── Start a new runner ───────────────────────────────────────────────────────

pub fn start_runner(
    store: Arc<StrategyRunnerStore>,
    config: RunnerConfig,
    workspace_dir: PathBuf,
    config_path: Option<PathBuf>,
) -> StoredRunner {
    let id = config.id.clone();
    let now = chrono::Utc::now().to_rfc3339();

    let status = RunnerStatus {
        id: id.clone(),
        status: "starting".to_string(),
        started_at: now.clone(),
        last_tick_at: None,
        next_tick_at: None,
        error: None,
    };

    let runner = StoredRunner { config: config.clone(), status: status.clone(), result: None };
    store.upsert(runner.clone());

    let store_clone = store.clone();
    let ws_dir = workspace_dir.clone();
    let task = tokio::spawn(async move {
        runner_loop(store_clone, config, ws_dir, config_path).await;
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
    // Dispatch to the correct loop based on market type
    if config.market_type == "polymarket_binary" {
        polymarket_runner_loop(store, config, workspace_dir, config_path).await;
    } else {
        crypto_runner_loop(store, config, workspace_dir).await;
    }
}

async fn crypto_runner_loop(
    store: Arc<StrategyRunnerStore>,
    config: RunnerConfig,
    workspace_dir: PathBuf,
) {
    let id = config.id.clone();

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

        buffer.push_back(candle);
        if buffer.len() > MAX_BUFFER { buffer.pop_front(); }

        // Run strategy on the current rolling window
        let metrics = crate::tools::backtest::run_rhai_on_candle_buffer(
            &script_content,
            buffer.iter().cloned().collect(),
            config.initial_balance,
            config.fee_pct,
        );

        // Update status timestamps
        let now = chrono::Utc::now().to_rfc3339();
        {
            let mut map = store.runners.lock().unwrap();
            if let Some(r) = map.get_mut(&id) {
                r.status.last_tick_at = Some(now);
                r.status.next_tick_at = None; // event-driven; no fixed next tick
            }
        }
        update_runner_result(&store, &id, &config, &metrics, None, None, None, None, None, None, None).await;
        store.persist();
    }

    // Channel closed means the feed task was dropped (runner stopped)
    tracing::info!("[RUNNER {id}] Feed channel closed, exiting");
}

// ── Polymarket binary live runner ─────────────────────────────────────────────
//
// For polymarket_binary, each window (5m/15m/1h/24h) is a separate market.
// Paper mode: uses a 1m Binance WebSocket feed and re-runs full slug simulation
//             at each window boundary to generate a fresh decision.
// Live mode:  same signal generation, but then calls Polymarket CLOB API to
//             place/cancel real orders. Verifies credentials and USDC balance first.

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

    // ─ Live mode: validate credentials/token and build CLOB client
    let mut clob_client: Option<std::sync::Arc<polymarket_trader::orders::ClobClient>> = if is_live {
        if config.poly_token_id.as_deref().unwrap_or("").trim().is_empty() {
            set_runner_error(
                &store,
                &id,
                "Live mode cannot start: no Polymarket token resolved for this series. Recreate the strategy and ensure a valid market series is selected.",
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
    let mut ticker_rx = crate::live_feed::spawn_binance_ticker_feed(binance_sym.clone());
    let store_for_ticker = store.clone();
    let id_for_ticker = id.clone();
    tokio::spawn(async move {
        while let Some(price) = ticker_rx.recv().await {
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
    // (window_ts, live_signal, bt_preview_signal, debug)
    // bt_preview_signal is captured at decision time (stateless, no capital constraint)
    // and used for signal comparison so a depleted BT balance doesn't cause false DISCREPANCYs.
    let mut prev_live_position: Option<(i64, String, String, std::collections::HashMap<String, f64>)> = None;
    let mut price_history: std::collections::VecDeque<(i64, f64)> = std::collections::VecDeque::with_capacity(60);
    // Persistent kv state for live signal — carries avg_vol and other ctx.set() values across windows.
    // Pre-seed from a BT warmup run so avg_vol starts aligned with the backtester's historical value,
    // preventing score divergence on the first windows after runner start.
    let mut live_kv_state: std::collections::HashMap<String, f64> = {
        let init_decision_minute = (window_minutes as i64) - 2;
        let res_logic = config.resolution_logic.as_deref().unwrap_or("price_up");
        match crate::tools::backtest::run_polymarket_bt_signal_preview(
            &script_content,
            buffer.iter().cloned().collect(),
            window_minutes,
            Some(init_decision_minute),
            res_logic,
            config.threshold,
            config.initial_balance,
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
    };
    // Live-mode counters (reset on runner start)
    let mut live_orders: Vec<LiveOrder> = Vec::new();
    let mut live_wins: u32 = 0;
    let mut live_total_trades: u32 = 0;

    // Minute within the window to take the decision (0-based index).
    // For a 5m window: decision at index 3 (the 4th candle, arriving at :34).
    // This gives a full minute (:34-:35) for the order to execute before resolution.
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

    loop {
        // Either a new closed candle arrives, or the early-fire timer fires.
        let early_fired = tokio::select! {
            candle_opt = candle_rx.recv() => {
                let live_inner = match candle_opt { Some(c) => c, None => break };
                // Shadow `live` for the rest of the block
                let live = live_inner;

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

                false // not an early fire event
            }
            _ = &mut early_sleep, if early_fire_armed_window != -1 && early_fire_armed_window != last_decision_window => {
                // Reset timer to far future so it doesn't keep firing
                early_sleep.as_mut().reset(tokio::time::Instant::now() + std::time::Duration::from_secs(86400 * 365));
                tracing::info!("[RUNNER {id}] Early fire triggered for window {}", early_fire_armed_window);
                append_runner_log(&store, &id, "Early fire: placing order before candle close");
                true // signal that this is an early-fire tick
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
        if current_window != last_window {
            last_window = current_window;
            tracing::info!(
                "[RUNNER {id}] New {}-min window @ {} UTC",
                window_minutes,
                chrono::DateTime::from_timestamp(current_window, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_default()
            );

            // Resolve tokens for the new window
            if let Some(ref series_id) = config.series_id {
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
                            // Fetch and cache historical trades for this market window
                            let ws = workspace_dir.clone();
                            let cond = condition_id.clone();
                            tokio::spawn(async move {
                                update_trade_cache(&cond, &ws).await;
                            });
                        }
                        Err(e) => {
                            tracing::error!(
                                "[RUNNER {id}] Failed to resolve token_id for window {}: {}",
                                current_window, e
                            );
                            append_runner_log(
                                &store, &id,
                                &format!("Token resolution failed for window {}: {}", current_window, e),
                            );
                        }
                    }
                }

            // Run backtest on the full buffer so we can compare backtest vs live
            // for the window that just completed.
            let metrics = eval_polymarket(&script_content, &buffer, window_minutes, &config);

            // Resolve previous window outcome and compare with backtest
            if let Some((prev_window, prev_signal, prev_bt_signal, prev_debug)) = prev_live_position.take() {
                    let res_logic = config.resolution_logic.as_deref().unwrap_or("price_up");
                    if let Some(went_up) = resolve_window_outcome(
                        &buffer, prev_window, window_secs as i64, res_logic, config.threshold,
                    ) {
                        let outcome = if went_up { "UP" } else { "DOWN" };

                        // ── Backtest vs Live comparison for this window ──
                        let decision_ts = prev_window + ((window_minutes as i64) - 2) * 60;
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
                            // Only count this as a trade if an order was actually placed for this window.
                            let has_order = live_orders.iter().any(|o| o.window_ts == prev_window);
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
                                        let ep = order.entry_price.unwrap_or(0.5).max(0.001);
                                        order.result = Some(result.to_string());
                                        order.pnl = Some(if won {
                                            order.amount_usdc * (1.0 / ep - 1.0)
                                        } else {
                                            -order.amount_usdc
                                        });
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
                fetch_usdc_balance_clob(client).await
            } else {
                Some(config.initial_balance)
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

            // Live signal: run script on the CURRENT (incomplete) window's decision candle.
            // This is NOT a backtest — it extracts buy/sell intent for the live market.
            let live_result = match crate::tools::backtest::run_polymarket_live_signal(
                &script_content,
                buffer.iter().cloned().collect(),
                window_minutes,
                Some(decision_minute),
                yes_token_price,
                &live_kv_state,
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

            let current_signal = live_result.signal.clone();
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
                let (order_result, renewed_client) = execute_live_polymarket_signal(
                    &id, clob_client.clone(), &current_signal, live_result.size, &config, &live, &store, current_window,
                    yes_token_price, no_token_price,
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
                }
            }

            // Metrics from backtest (historical) are still useful for display
            let metrics = eval_polymarket(&script_content, &buffer, window_minutes, &config);
            let wallet_balance = if let Some(ref client) = clob_client {
                fetch_usdc_balance_clob(client).await
            } else {
                Some(config.initial_balance)
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
    tracing::info!("[RUNNER {id}] Polymarket feed closed, exiting");
}

/// Resolve both YES and NO token IDs for the current window slug.
async fn resolve_token_for_window(series_id: &str, window_ts: u64) -> anyhow::Result<(String, String, String)> {
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
async fn execute_live_polymarket_signal(
    id: &str,
    client: Option<Arc<polymarket_trader::orders::ClobClient>>,
    signal: &str,
    script_frac: f64,
    config: &RunnerConfig,
    _live: &crate::live_feed::LiveCandle,
    store: &Arc<StrategyRunnerStore>,
    window_ts: i64,
    yes_token_price: f64,
    no_token_price: f64,
) -> (Option<LiveOrder>, Option<Arc<polymarket_trader::orders::ClobClient>>) {
    use polymarket_trader::orders::Side;

    // In binary markets YES/NO are complementary tokens.
    // "yes" → buy YES token  |  "no" → buy NO token
    let (token_id, side) = if signal.starts_with("yes") {
        match &config.poly_token_id {
            Some(tid) if !tid.is_empty() => (tid.clone(), Side::Buy),
            _ => {
                tracing::warn!("[RUNNER {id}] Live mode: no YES token_id configured, skipping order");
                append_runner_log(store, id, "No Polymarket YES token_id configured.");
                return (None, None);
            }
        }
    } else if signal.starts_with("no") {
        match &config.poly_no_token_id {
            Some(tid) if !tid.is_empty() => (tid.clone(), Side::Buy),
            _ => {
                tracing::warn!("[RUNNER {id}] Live mode: no NO token_id configured, skipping order");
                append_runner_log(store, id, "No Polymarket NO token_id configured.");
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
                tracing::warn!("[RUNNER {id}] Cannot place order: zero or unknown USDC balance");
                append_runner_log(store, id, "Skipped: zero USDC balance");
                return (None, None);
            }
            let amt = match config.live_sizing_mode {
                LiveSizingMode::Fixed => {
                    let amt = config.live_sizing_value.max(5.0).round();
                    tracing::info!("[RUNNER {id}] Sizing (fixed): ${:.0}", amt);
                    amt
                }
                LiveSizingMode::Percent => {
                    let max_frac = config.live_sizing_value.max(0.0).min(1.0);
                    let frac = script_frac.clamp(0.0, max_frac);
                    let amt = (bal * frac).max(5.0).round();
                    tracing::info!(
                        "[RUNNER {id}] Sizing (percent): balance=${:.2} script_frac={:.4} max_frac={:.4} amount=${:.0}",
                        bal, script_frac, max_frac, amt
                    );
                    amt
                }
            };
            (bal, amt)
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
                    let max_frac = config.live_sizing_value.max(0.0).min(1.0);
                    let frac = script_frac.clamp(0.0, max_frac);
                    let amt = (bal * frac).max(5.0).round().min(bal);
                    tracing::info!(
                        "[RUNNER {id}] Paper sizing (percent): balance=${:.2} script_frac={:.4} max_frac={:.4} amount=${:.0}",
                        bal, script_frac, max_frac, amt
                    );
                    amt
                }
            };
            if amt <= 0.0 || amt > bal {
                tracing::warn!("[RUNNER {id}] Paper order: insufficient balance ${:.2} for amount ${:.0}", bal, amt);
                append_runner_log(store, id, &format!("Skipped: insufficient paper balance ${:.2}", bal));
                return (None, None);
            }
            (bal, amt)
        }
    };

    let ep = if signal.starts_with("yes") { yes_token_price } else { no_token_price };

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

        let worst_price = 0.0_f64;
        let max_retries = 3;
        let retry_delay = std::time::Duration::from_secs(10);
        let mut attempt = 0;

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
                "[RUNNER {id}] Live MARKET order (attempt {}/{}): {:?} {} USDC on token {}",
                attempt, max_retries, side, amount_usdc, token_id
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
                            // Re-authenticate via L1 EIP-712 to get fresh L2 session.
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
        // ── PAPER mode: simulate order ────────────────────────────
        let order_id = format!("paper-{}", chrono::Utc::now().timestamp_millis());
        tracing::info!(
            "[RUNNER {id}] Paper order: {} ${:.0} on token {} @ {:.4}",
            signal, amount_usdc, token_id, ep
        );
        append_runner_log(
            store, id,
            &format!("Paper order: {} ${:.0} @ {:.4} (id={})", signal, amount_usdc, ep, order_id),
        );
        (Some(LiveOrder {
            timestamp: chrono::Utc::now().to_rfc3339(),
            window_ts,
            side: signal.to_string(),
            token_id,
            amount_usdc,
            order_id,
            status: "LIVE".to_string(),
            entry_price: Some(ep),
            result: None,
            pnl: None,
            stop_loss_triggered: false,
        }), None)
    }
}

/// Fetch Yes/No token prices from Polymarket CLOB API.
/// Returns (yes_price, no_price) or (0.0, 0.0) on error.
async fn fetch_token_prices(yes_token_id: &str, no_token_id: &str) -> (f64, f64) {
    let client = reqwest::Client::new();

    let yes_url = format!("https://clob.polymarket.com/price?token_id={}&side=buy", yes_token_id);
    let yes_price = match client.get(&yes_url).timeout(std::time::Duration::from_secs(5)).send().await {
        Ok(resp) if resp.status().is_success() => {
            resp.json::<serde_json::Value>().await
                .ok()
                .and_then(|v| v.get("price").and_then(|p| p.as_str()).map(|s| s.to_string()))
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.0)
        }
        Ok(resp) => {
            tracing::warn!("[PRICE] YES token /price failed: {} for token {}", resp.status(), yes_token_id);
            0.0
        }
        Err(e) => {
            tracing::warn!("[PRICE] YES token /price request error: {} for token {}", e, yes_token_id);
            0.0
        }
    };

    let no_url = format!("https://clob.polymarket.com/price?token_id={}&side=buy", no_token_id);
    let no_price = match client.get(&no_url).timeout(std::time::Duration::from_secs(5)).send().await {
        Ok(resp) if resp.status().is_success() => {
            resp.json::<serde_json::Value>().await
                .ok()
                .and_then(|v| v.get("price").and_then(|p| p.as_str()).map(|s| s.to_string()))
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.0)
        }
        Ok(resp) => {
            tracing::warn!("[PRICE] NO token /price failed: {} for token {}", resp.status(), no_token_id);
            0.0
        }
        Err(e) => {
            tracing::warn!("[PRICE] NO token /price request error: {} for token {}", e, no_token_id);
            0.0
        }
    };

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
async fn persist_polymarket_creds(
    config_path: &std::path::Path,
    creds: &polymarket_trader::auth::PolyCredentials,
) -> anyhow::Result<()> {
    let raw = tokio::fs::read_to_string(config_path).await?;
    let mut doc: toml::Value = raw.parse()?;
    if let Some(pm) = doc.get_mut("polymarket").and_then(|v| v.as_table_mut()) {
        pm.insert("api_key".to_string(), toml::Value::String(creds.api_key.clone()));
        pm.insert("secret".to_string(), toml::Value::String(creds.secret.clone()));
        pm.insert("passphrase".to_string(), toml::Value::String(creds.passphrase.clone()));
    }
    let updated = toml::to_string_pretty(&doc)?;
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

fn set_runner_error(store: &Arc<StrategyRunnerStore>, id: &str, msg: &str) {
    tracing::error!("[RUNNER {id}] Error: {msg}");
    let mut map = store.runners.lock().unwrap();
    if let Some(r) = map.get_mut(id) {
        r.status.status = "error".to_string();
        r.status.error  = Some(msg.to_string());
    }
    drop(map);
    store.persist();
}

fn set_runner_status(store: &Arc<StrategyRunnerStore>, id: &str, status: &str) {
    let mut map = store.runners.lock().unwrap();
    if let Some(r) = map.get_mut(id) {
        r.status.status = status.to_string();
    }
    drop(map);
    store.persist();
}

/// Fetch USDC trading balance from Polymarket CLOB API.
async fn fetch_usdc_balance_clob(client: &polymarket_trader::orders::ClobClient) -> Option<f64> {
    // Prefer CLOB API balance (always works as long as L2 auth is valid).
    // Fall back to Polygon RPC only if the API call fails.
    match client.get_api_balance().await {
        Ok(api_bal) => {
            tracing::info!("Polymarket CLOB API balance: ${:.2}", api_bal);
            return Some(api_bal);
        }
        Err(e) => {
            tracing::warn!("Failed to fetch Polymarket CLOB API balance: {e}");
        }
    }

    match client.get_balance().await {
        Ok(bal) => {
            tracing::info!("Polymarket CLOB RPC balance: ${:.2}", bal);
            Some(bal)
        }
        Err(e) => {
            tracing::warn!("Failed to fetch Polymarket CLOB RPC balance: {e}");
            None
        }
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

    // Preserve existing live counters/orders if not explicitly provided
    let (orders, wins, total) = {
        let map = store.runners.lock().unwrap();
        if let Some(ref existing) = map.get(id).and_then(|r| r.result.as_ref()) {
            (
                live_orders.unwrap_or_else(|| existing.live_orders.clone()),
                live_wins.unwrap_or(existing.live_wins),
                live_total_trades.unwrap_or(existing.live_total_trades),
            )
        } else {
            (live_orders.unwrap_or_default(), live_wins.unwrap_or(0), live_total_trades.unwrap_or(0))
        }
    };

    let result = RunnerResult {
        total_return_pct: metrics.total_return_pct,
        balance: config.initial_balance * (1.0 + metrics.total_return_pct / 100.0),
        position: 0.0,
        total_trades: metrics.total_trades,
        win_rate_pct: metrics.win_rate_pct,
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

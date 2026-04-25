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

    pub fn restart_previously_running(self: &Arc<Self>, workspace_dir: PathBuf) -> usize {
        let configs = self.list_restartable_configs();
        let count = configs.len();
        for config in configs {
            let id = config.id.clone();
            if self.handles.lock().unwrap().contains_key(&id) {
                continue;
            }
            let store = self.clone();
            let ws_dir = workspace_dir.clone();
            let task = tokio::spawn(async move {
                runner_loop(store, config, ws_dir).await;
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
        runner_loop(store_clone, config, ws_dir).await;
    });
    store.register_handle(id, task.abort_handle());

    runner
}

// ── Background runner loop ────────────────────────────────────────────────────

async fn runner_loop(
    store: Arc<StrategyRunnerStore>,
    config: RunnerConfig,
    workspace_dir: PathBuf,
) {
    // Dispatch to the correct loop based on market type
    if config.market_type == "polymarket_binary" {
        polymarket_runner_loop(store, config, workspace_dir).await;
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
    config: RunnerConfig,
    workspace_dir: PathBuf,
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
    let clob_client: Option<std::sync::Arc<polymarket_trader::orders::ClobClient>> = if is_live {
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

    let mut last_window: i64 = -1;
    let mut last_decision_window: i64 = -1;
    let mut prev_live_position: Option<(i64, String)> = None; // (window_ts, signal)
    let mut price_history: std::collections::VecDeque<(i64, f64)> = std::collections::VecDeque::with_capacity(60);
    // Live-mode counters (reset on runner start)
    let mut live_orders: Vec<LiveOrder> = Vec::new();
    let mut live_wins: u32 = 0;
    let mut live_total_trades: u32 = 0;

    // Minute within the window to take the decision.
    // For a 5m window: minute 0 starts at :00, decision at minute 4 (:04 candle close),
    // resolution at minute 5 when the next window begins.
    let decision_minute = (window_minutes as i64) - 1;

    while let Some(live) = candle_rx.recv().await {
        let candle = crate::tools::backtest::Candle {
            open_time_ms: live.open_time_ms,
            open: live.open, high: live.high, low: live.low,
            close: live.close, volume: live.volume,
        };

        buffer.push_back(candle.clone());
        if buffer.len() > MAX_BUFFER { buffer.pop_front(); }

        // Track price history for live chart (keep last 60 points)
        price_history.push_back((live.open_time_ms, live.close));
        if price_history.len() > 60 { price_history.pop_front(); }

        // Window this candle belongs to.
        // Use candle *close* time for window detection so the new window is
        // recognised immediately when the last 1m candle of the previous window
        // closes (eliminates the ~1min inherent delay).
        let candle_close_ts_secs = (live.open_time_ms / 1000) + 60;
        let current_window = candle_close_ts_secs - (candle_close_ts_secs % window_secs as i64);
        let next_window = current_window + window_secs as i64;
        let window_seconds_left = next_window - candle_close_ts_secs;
        // Minute within the current window (0-based from close time).
        // For a 5m window: minute 0..4 correspond to candles 0..4.
        let minute_in_window = (candle_close_ts_secs % window_secs as i64) / 60;

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

            // Live mode: resolve tokens for the new window
            if is_live {
                if let Some(ref series_id) = config.series_id {
                    match resolve_token_for_window(series_id, current_window as u64).await {
                        Ok((yes_id, no_id)) => {
                            tracing::info!(
                                "[RUNNER {id}] Resolved tokens for window {}: YES={} NO={}",
                                current_window, yes_id, no_id
                            );
                            let mut map = store.runners.lock().unwrap();
                            if let Some(r) = map.get_mut(&id) {
                                r.config.poly_token_id = Some(yes_id);
                                r.config.poly_no_token_id = Some(no_id);
                            }
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
            }

            // Resolve previous window outcome
            if is_live {
                if let Some((prev_window, prev_signal)) = prev_live_position.take() {
                    let res_logic = config.resolution_logic.as_deref().unwrap_or("price_up");
                    if let Some(went_up) = resolve_window_outcome(
                        &buffer, prev_window, window_secs as i64, res_logic, config.threshold,
                    ) {
                        let won = (prev_signal.starts_with("yes") && went_up)
                            || (prev_signal.starts_with("no") && !went_up);
                        live_total_trades += 1;
                        if won { live_wins += 1; }
                        let outcome = if went_up { "UP" } else { "DOWN" };
                        let pos = if prev_signal.starts_with("yes") { "YES" } else { "NO" };
                        let result = if won { "WIN" } else { "LOSS" };
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
            let wallet_balance = if is_live {
                if let Some(ref client) = clob_client {
                    fetch_usdc_balance_clob(client).await
                } else { None }
            } else { None };
            let metrics = eval_polymarket(&script_content, &buffer, window_minutes, &config);
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
        if is_live && minute_in_window == decision_minute && current_window != last_decision_window {
            last_decision_window = current_window;
            tracing::info!(
                "[RUNNER {id}] Decision point for window {} (minute {}), evaluating strategy...",
                current_window, decision_minute
            );

            // Live signal: run script on the CURRENT (incomplete) window's decision candle.
            // This is NOT a backtest — it extracts buy/sell intent for the live market.
            let current_signal = match crate::tools::backtest::run_polymarket_live_signal(
                &script_content,
                buffer.iter().cloned().collect(),
                window_minutes,
            ) {
                Ok(sig) => {
                    tracing::info!("[RUNNER {id}] Live signal for window {}: {}", current_window, sig);
                    append_runner_log(&store, &id, &format!("Signal window {}: {}", current_window, sig));
                    sig
                }
                Err(e) => {
                    tracing::warn!("[RUNNER {id}] Live signal eval failed: {}", e);
                    append_runner_log(&store, &id, &format!("Signal eval FAILED: {}", e));
                    "flat".to_string()
                }
            };

            if !current_signal.starts_with("flat") {
                if let Some(ref client) = clob_client {
                    if let Some(order) = execute_live_polymarket_signal(
                        &id, client, &current_signal, &config, &live, &store, current_window,
                    ).await {
                        prev_live_position = Some((current_window, current_signal.clone()));
                        live_orders.push(order);
                    }
                }
            } else {
                append_runner_log(&store, &id, "Signal: flat (no trade)");
            }

            // Metrics from backtest (historical) are still useful for display
            let metrics = eval_polymarket(&script_content, &buffer, window_minutes, &config);
            let wallet_balance = if let Some(ref client) = clob_client {
                fetch_usdc_balance_clob(client).await
            } else { None };
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
        if is_live {
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
    }
    tracing::info!("[RUNNER {id}] Polymarket feed closed, exiting");
}

/// Resolve both YES and NO token IDs for the current window slug.
async fn resolve_token_for_window(series_id: &str, window_ts: u64) -> anyhow::Result<(String, String)> {
    let series = crate::tools::series::builtin_series()
        .into_iter()
        .find(|s| s.id == series_id)
        .ok_or_else(|| anyhow::anyhow!("Selected Market Series is not recognized: {}", series_id))?;

    let target_slug = format!("{}-{}", series.slug_prefix, window_ts);

    let market = polymarket_trader::markets::get_market(&target_slug)
        .await
        .map_err(|e| anyhow::anyhow!(
            "No active Polymarket market found for slug {}. Error: {}",
            target_slug,
            e
        ))?;

    if market.yes_token_id.trim().is_empty() {
        anyhow::bail!("The selected market has no YES token yet for slug {}.", target_slug);
    }
    if market.no_token_id.trim().is_empty() {
        anyhow::bail!("The selected market has no NO token yet for slug {}.", target_slug);
    }

    Ok((market.yes_token_id, market.no_token_id))
}

/// Returns the LiveOrder if successfully placed.
async fn execute_live_polymarket_signal(
    id: &str,
    client: &polymarket_trader::orders::ClobClient,
    signal: &str,
    config: &RunnerConfig,
    _live: &crate::live_feed::LiveCandle,
    store: &Arc<StrategyRunnerStore>,
    window_ts: i64,
) -> Option<LiveOrder> {
    use polymarket_trader::orders::Side;

    // In binary markets YES/NO are complementary tokens.
    // "yes" → buy YES token  |  "no" → buy NO token
    let (token_id, side) = if signal.starts_with("yes") {
        match &config.poly_token_id {
            Some(tid) if !tid.is_empty() => (tid.clone(), Side::Buy),
            _ => {
                tracing::warn!("[RUNNER {id}] Live mode: no YES token_id configured, skipping order");
                append_runner_log(store, id, "No Polymarket YES token_id configured.");
                return None;
            }
        }
    } else if signal.starts_with("no") {
        match &config.poly_no_token_id {
            Some(tid) if !tid.is_empty() => (tid.clone(), Side::Buy),
            _ => {
                tracing::warn!("[RUNNER {id}] Live mode: no NO token_id configured, skipping order");
                append_runner_log(store, id, "No Polymarket NO token_id configured.");
                return None;
            }
        }
    } else {
        tracing::debug!("[RUNNER {id}] Signal '{signal}' — no order placed");
        return None;
    };

    // Use a fixed position size: $10 USDC per trade (safe default)
    let amount_usdc = 10.0_f64;
    // Market order: let the SDK calculate the real share price from the
    // orderbook.  Polymarket share prices are 0–1 (probabilities), not raw
    // crypto prices like $78k BTC, so live.close is not a valid worst_price.
    let worst_price = 0.0_f64;

    tracing::info!(
        "[RUNNER {id}] Live order: {:?} {} USDC on token {} @ ~{:.4}",
        side, amount_usdc, token_id, worst_price
    );

    match client.create_market_order(&token_id, side, amount_usdc, worst_price).await {
        Ok(resp) => {
            tracing::info!(
                "[RUNNER {id}] Order placed: id={} status={}",
                resp.order_id, resp.status
            );
            append_runner_log(
                store, id,
                &format!("Order placed: {} {} USDC (id={})", signal, amount_usdc, resp.order_id),
            );
            Some(LiveOrder {
                timestamp: chrono::Utc::now().to_rfc3339(),
                window_ts,
                side: signal.to_string(),
                token_id,
                amount_usdc,
                order_id: resp.order_id,
                status: resp.status,
            })
        }
        Err(e) => {
            let msg = e.to_string();
            tracing::error!("[RUNNER {id}] Order failed: {msg}");
            append_runner_log(store, id, &format!("Order error: {msg}"));
            // Don't abort the runner on a single failed order; just log it
            None
        }
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
        _ => 0.0,
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
        _ => 0.0,
    };

    (yes_price, no_price)
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
        // Keep last 5 log lines
        let lines: Vec<&str> = updated.lines().collect();
        let truncated = lines.iter().rev().take(5).rev().cloned().collect::<Vec<_>>().join("\n");
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
    // Extract from Polymarket slug (e.g. "btc-updown-5m-..." → BTCUSDT)
    let lower = symbol.to_lowercase();
    if lower.starts_with("btc") || lower.contains("btc") { return "BTCUSDT".to_string(); }
    if lower.starts_with("eth") || lower.contains("eth") { return "ETHUSDT".to_string(); }
    // Weather / other series: no Binance symbol; return as-is
    s
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

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

    pub fn set_poly_token_id(&self, id: &str, token_id: String) -> bool {
        let mut map = self.runners.lock().unwrap();
        if let Some(r) = map.get_mut(id) {
            r.config.poly_token_id = Some(token_id);
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

    // ─ Resolve script ─────────────────────────────────────────────────────────
    let script_path = {
        let p = std::path::Path::new(&config.script);
        if p.is_absolute() || p.exists() { p.to_path_buf() }
        else { workspace_dir.join("scripts").join(&config.script) }
    };
    if !script_path.exists() {
        set_runner_error(&store, &id, &format!("Script not found: {}", script_path.display()));
        return;
    }
    let script_content = match std::fs::read_to_string(&script_path) {
        Ok(s) => s,
        Err(e) => { set_runner_error(&store, &id, &e.to_string()); return; }
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
    update_runner_result(&store, &id, &config, &initial_metrics);
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
        update_runner_result(&store, &id, &config, &metrics);
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

    // ─ Resolve script
    let script_path = {
        let p = std::path::Path::new(&config.script);
        if p.is_absolute() || p.exists() { p.to_path_buf() }
        else { workspace_dir.join("scripts").join(&config.script) }
    };
    if !script_path.exists() {
        set_runner_error(&store, &id, &format!("Script not found: {}", script_path.display()));
        return;
    }
    let script_content = match std::fs::read_to_string(&script_path) {
        Ok(s) => s,
        Err(e) => { set_runner_error(&store, &id, &e.to_string()); return; }
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
    update_runner_result(&store, &id, &config, &initial);
    set_runner_status(&store, &id, "running");

    // ─ Connect 1m WebSocket (polymarket binary always uses 1m real-time feed)
    let mut candle_rx = crate::live_feed::spawn_binance_kline_feed(
        binance_sym.clone(),
        "1m".to_string(),
    );
    tracing::info!("[RUNNER {id}] Polymarket live feed started: {binance_sym}@1m (window={window_secs}s)");

    let mut last_eval_window: i64 = -1;
    let mut last_live_signal: String = "flat".to_string();

    while let Some(live) = candle_rx.recv().await {
        let candle = crate::tools::backtest::Candle {
            open_time_ms: live.open_time_ms,
            open: live.open, high: live.high, low: live.low,
            close: live.close, volume: live.volume,
        };

        buffer.push_back(candle.clone());
        if buffer.len() > MAX_BUFFER { buffer.pop_front(); }

        // Window this candle belongs to
        let candle_ts_secs = live.open_time_ms / 1000;
        let current_window = candle_ts_secs - (candle_ts_secs % window_secs as i64);

        // Re-evaluate at each new window boundary (i.e., once per window period)
        if current_window != last_eval_window {
            last_eval_window = current_window;
            let next_window = current_window + window_secs as i64;
            tracing::info!(
                "[RUNNER {id}] New {}-min window @ {} UTC, evaluating strategy...",
                window_minutes,
                chrono::DateTime::from_timestamp(current_window, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_default()
            );

            let metrics = eval_polymarket(&script_content, &buffer, window_minutes, &config);

            // ─ Live mode: act on signal change
            if is_live {
                let current_signal = metrics.all_trades.last()
                    .map(|t| t.side.clone())
                    .unwrap_or_else(|| "flat".to_string());

                if current_signal != last_live_signal {
                    if let Some(ref client) = clob_client {
                        execute_live_polymarket_signal(
                            &id,
                            client,
                            &current_signal,
                            &config,
                            &live,
                            &store,
                        ).await;
                        last_live_signal = current_signal;
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
            update_runner_result(&store, &id, &config, &metrics);
            store.persist();
        }
    }
    tracing::info!("[RUNNER {id}] Polymarket feed closed, exiting");
}

/// Execute a real Polymarket CLOB order based on the strategy signal.
/// - "buy"  → place a BUY (YES) market order for the configured position size.
/// - "sell" → place a SELL (NO) market order.
/// - "flat" / other → no action.
async fn execute_live_polymarket_signal(
    id: &str,
    client: &polymarket_trader::orders::ClobClient,
    signal: &str,
    config: &RunnerConfig,
    live: &crate::live_feed::LiveCandle,
    store: &Arc<StrategyRunnerStore>,
) {
    use polymarket_trader::orders::Side;

    let side = match signal {
        "buy"  => Side::Buy,
        "sell" => Side::Sell,
        _ => {
            tracing::debug!("[RUNNER {id}] Signal '{signal}' — no order placed");
            return;
        }
    };

    // Resolve token_id from config or a sensible default
    let token_id = match &config.poly_token_id {
        Some(tid) if !tid.is_empty() => tid.clone(),
        _ => {
            // Cannot place order without token_id; log and skip
            tracing::warn!("[RUNNER {id}] Live mode: no token_id configured, skipping order");
            append_runner_log(
                store, id,
                "No Polymarket token_id configured. Set the market token ID to enable live orders.",
            );
            return;
        }
    };

    // Use a fixed position size: $10 USDC per trade (safe default)
    // In future this could come from a config field (e.g. max_position_usd)
    let amount_usdc = 10.0_f64;
    let worst_price = live.close; // use current close as price reference

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
                &format!("Live order placed: {:?} ${amount_usdc} — order_id={}", side, resp.order_id),
            );
        }
        Err(e) => {
            let msg = format!("Live order failed: {e}");
            tracing::error!("[RUNNER {id}] {msg}");
            append_runner_log(store, id, &format!("Order error: {msg}"));
            // Don't abort the runner on a single failed order; just log it
        }
    }
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

fn update_runner_result(
    store: &Arc<StrategyRunnerStore>,
    id: &str,
    config: &RunnerConfig,
    metrics: &crate::tools::backtest::BacktestMetrics,
) {
    let last_signal = metrics.all_trades.last()
        .map(|t| t.side.clone())
        .unwrap_or_else(|| "flat".to_string());
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

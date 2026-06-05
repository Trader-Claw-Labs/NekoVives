//! 1-Hz tick recorder for Polymarket UP/DOWN binary markets.
//!
//! Records every second:
//!   - Polymarket CLOB YES/NO best-bid / best-ask for a given condition_id
//!   - Binance spot price (via miniTicker WebSocket, updated every ~1 s)
//!   - Chainlink oracle price (via REST poller, when configured)
//!   - Computed oracle_lag_ms (wall-clock delay between Binance and Chainlink)
//!
//! Output: newline-delimited JSON records flushed to a JSONL file that rotates
//! daily, stored under `<workspace>/data/ticks/<slug>/<YYYY-MM-DD>.jsonl`.
//! Files accumulate for 7 days then are pruned by the recorder itself.
//!
//! Usage (from strategy_runner or a gateway route):
//! ```rust
//! let handle = TickRecorder::start(TickRecorderConfig { ... });
//! // ... later:
//! handle.stop();
//! ```

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::io::AsyncWriteExt;

// ── Tick record ──────────────────────────────────────────────────────────────

/// One second of market data for a single Polymarket binary market.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct Tick {
    /// Unix timestamp in milliseconds (wall clock).
    pub ts_ms: i64,
    /// Polymarket YES token best-bid price (0-1), or 0 if unavailable.
    pub yes_bid: f64,
    /// Polymarket YES token best-ask price (0-1), or 0 if unavailable.
    pub yes_ask: f64,
    /// Polymarket NO token best-bid price (0-1), or 0 if unavailable.
    pub no_bid: f64,
    /// Polymarket NO token best-ask price (0-1), or 0 if unavailable.
    pub no_ask: f64,
    /// YES mid-price = (yes_bid + yes_ask) / 2, or 0 if unavailable.
    pub yes_mid: f64,
    /// Last Binance spot price updated by the miniTicker WebSocket (0 if feed not started).
    pub binance_price: f64,
    /// Latest Chainlink oracle price (0 if not configured or not yet fetched).
    pub chainlink_price: f64,
    /// Wall-clock lag between the Binance trade timestamp and the Chainlink update
    /// (milliseconds). Negative means Chainlink leads Binance (shouldn't happen).
    /// 0 if either feed is unavailable.
    pub oracle_lag_ms: i64,
    /// Active 5-min window start timestamp (Unix seconds), aligns to 300s boundaries.
    pub window_ts: i64,
    /// Seconds remaining in the current 5-min window.
    pub window_secs_left: i64,

    // ── Order-book depth fields (0 when not available) ────────────────────────
    /// USD-equivalent liquidity on the YES ask side within 2% of best ask.
    /// Populated by the live tick recorder from CLOB /book every 10 s.
    /// Used by the on_tick backtester to simulate realistic market-order fill.
    #[serde(default)]
    pub ask_depth_usd: f64,
    /// USD-equivalent liquidity on the YES bid side within 2% of best bid.
    #[serde(default)]
    pub bid_depth_usd: f64,

    // ── Official Polymarket resolution (populated by to-ticks from Gamma API) ─
    /// True = YES won (price went UP), False = NO won, None = not resolved yet.
    /// Written by `orderbook_parser.py to-ticks` after querying gamma-api.polymarket.com.
    /// When present, the on_tick backtester uses this for exact resolution instead of
    /// comparing Binance prices (avoids oracle timing mismatch).
    #[serde(default)]
    pub window_yes_won: Option<bool>,
}

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct TickRecorderConfig {
    /// Polymarket condition_id (hex token id) or market slug for display.
    pub condition_id: String,
    /// Short slug used in the output file path (e.g. "btc_5m").
    pub slug: String,
    /// Binance symbol to track (e.g. "BTCUSDT").
    pub binance_symbol: String,
    /// Optional Chainlink REST endpoint URL.
    pub chainlink_url: Option<String>,
    /// Optional Chainlink API key (Bearer).
    pub chainlink_api_key: Option<String>,
    /// Polymarket CLOB base URL (default: "https://clob.polymarket.com").
    pub clob_base_url: String,
    /// Directory to write JSONL files.
    pub output_dir: PathBuf,
    /// How many calendar days of JSONL files to keep before pruning.
    pub retain_days: u64,
    /// Polymarket gamma API base URL for price queries
    pub gamma_base_url: String,
}

impl TickRecorderConfig {
    pub fn new(
        slug: impl Into<String>,
        condition_id: impl Into<String>,
        binance_symbol: impl Into<String>,
        workspace_dir: &Path,
    ) -> Self {
        let slug = slug.into();
        Self {
            condition_id: condition_id.into(),
            slug: slug.clone(),
            binance_symbol: binance_symbol.into(),
            chainlink_url: None,
            chainlink_api_key: None,
            clob_base_url: "https://clob.polymarket.com".to_string(),
            gamma_base_url: "https://gamma-api.polymarket.com".to_string(),
            output_dir: workspace_dir.join("data").join("ticks").join(&slug),
            retain_days: 7,
        }
    }
}

// ── Handle ────────────────────────────────────────────────────────────────────

/// Opaque handle to a running tick recorder. Drop it or call `stop()` to halt.
pub struct TickRecorderHandle {
    stop_flag: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

impl TickRecorderHandle {
    /// Signal the background task to stop gracefully.
    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
    }

    pub fn is_running(&self) -> bool {
        !self.stop_flag.load(Ordering::SeqCst) && !self.task.is_finished()
    }
}

impl Drop for TickRecorderHandle {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
    }
}

// ── Recorder ──────────────────────────────────────────────────────────────────

pub struct TickRecorder;

impl TickRecorder {
    /// Start the tick recorder. Returns a handle; stop it with `handle.stop()`.
    pub fn start(cfg: TickRecorderConfig) -> TickRecorderHandle {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let flag = stop_flag.clone();

        let task = tokio::spawn(async move {
            if let Err(e) = recorder_loop(cfg, flag).await {
                tracing::error!("[TICK_RECORDER] Fatal error: {e}");
            }
        });

        TickRecorderHandle { stop_flag, task }
    }
}

// ── Internal loop ─────────────────────────────────────────────────────────────

async fn recorder_loop(cfg: TickRecorderConfig, stop: Arc<AtomicBool>) -> anyhow::Result<()> {
    // Create output directory
    tokio::fs::create_dir_all(&cfg.output_dir).await?;
    tracing::info!(
        "[TICK_RECORDER] Starting — slug={} symbol={} dir={}",
        cfg.slug,
        cfg.binance_symbol,
        cfg.output_dir.display()
    );

    // ── Shared state for the live Binance price ───────────────────────────────
    let binance_price = Arc::new(std::sync::RwLock::new(0f64));
    let bp_write = binance_price.clone();

    // Binance miniTicker WebSocket — updates every ~1s on trade activity
    let mut ticker_rx = crate::live_feed::spawn_binance_ticker_feed(cfg.binance_symbol.clone());
    let stop_ticker = stop.clone();
    tokio::spawn(async move {
        loop {
            if stop_ticker.load(Ordering::SeqCst) {
                break;
            }
            match ticker_rx.recv().await {
                Some(price) => {
                    *bp_write.write().unwrap() = price;
                }
                None => break, // channel closed → feed restarted internally
            }
        }
    });

    // ── Optional Chainlink price handle ──────────────────────────────────────
    // We store (price, last_update_ms) together.
    let chainlink_state: Arc<std::sync::RwLock<(f64, i64)>> =
        Arc::new(std::sync::RwLock::new((0.0, 0)));
    let cl_state_write = chainlink_state.clone();

    if let Some(ref url) = cfg.chainlink_url {
        let url = url.clone();
        let api_key = cfg.chainlink_api_key.clone();
        let stop_cl = stop.clone();
        tokio::spawn(async move {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_default();
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
            loop {
                interval.tick().await;
                if stop_cl.load(Ordering::SeqCst) {
                    break;
                }
                let mut req = client.get(&url);
                if let Some(ref key) = api_key {
                    req = req.header("Authorization", format!("Bearer {key}"));
                }
                if let Ok(resp) = req.send().await {
                    if resp.status().is_success() {
                        if let Ok(json) = resp.json::<serde_json::Value>().await {
                            if let Some(price) = extract_price(&json) {
                                let now_ms = chrono::Utc::now().timestamp_millis();
                                *cl_state_write.write().unwrap() = (price, now_ms);
                            }
                        }
                    }
                }
            }
        });
    }

    // ── HTTP client for Polymarket CLOB price polling ─────────────────────────
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;

    // ── 1-Hz tick loop ────────────────────────────────────────────────────────
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
    let window_secs: i64 = 300; // 5-minute Polymarket windows

    // Track current JSONL file to avoid re-opening every second
    let mut current_day = String::new();
    let mut file: Option<tokio::fs::File> = None;

    // Cached book depth: refreshed every 10 seconds from CLOB /book
    let mut cached_ask_depth_usd: f64 = 0.0;
    let mut cached_bid_depth_usd: f64 = 0.0;
    let mut depth_tick_counter: u32 = 0;

    loop {
        interval.tick().await;

        if stop.load(Ordering::SeqCst) {
            tracing::info!("[TICK_RECORDER] Stopped ({})", cfg.slug);
            break;
        }

        let now_ms = chrono::Utc::now().timestamp_millis();
        let now_s = now_ms / 1000;

        // ── Roll file at midnight ─────────────────────────────────────────────
        let day_str = chrono::Utc::now().format("%Y-%m-%d").to_string();
        if day_str != current_day {
            // Flush old file
            if let Some(ref mut f) = file {
                let _ = f.flush().await;
            }
            // Prune old files
            prune_old_files(&cfg.output_dir, cfg.retain_days).await;
            // Open new file (append mode)
            let path = cfg.output_dir.join(format!("{day_str}.jsonl"));
            match tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .await
            {
                Ok(f) => {
                    tracing::info!("[TICK_RECORDER] Opened {}", path.display());
                    file = Some(f);
                    current_day = day_str;
                }
                Err(e) => {
                    tracing::warn!("[TICK_RECORDER] Cannot open output file: {e}");
                    continue;
                }
            }
        }

        // ── Refresh order-book depth every 10 s ──────────────────────────────
        depth_tick_counter += 1;
        if depth_tick_counter >= 10 {
            depth_tick_counter = 0;
            let (ad, bd) = fetch_book_depth(&http, &cfg.clob_base_url, &cfg.condition_id).await;
            if ad > 0.0 { cached_ask_depth_usd = ad; }
            if bd > 0.0 { cached_bid_depth_usd = bd; }
        }

        // ── Fetch Polymarket CLOB prices ──────────────────────────────────────
        let (yes_bid, yes_ask, no_bid, no_ask) =
            fetch_clob_prices(&http, &cfg.clob_base_url, &cfg.condition_id).await;
        let yes_mid = if yes_bid > 0.0 && yes_ask > 0.0 {
            (yes_bid + yes_ask) / 2.0
        } else if yes_bid > 0.0 {
            yes_bid
        } else {
            yes_ask
        };

        // ── Read shared prices ────────────────────────────────────────────────
        let binance_price_now = *binance_price.read().unwrap();
        let (chainlink_price_now, chainlink_update_ms) = *chainlink_state.read().unwrap();

        // Oracle lag: time since Chainlink was last updated
        let oracle_lag_ms = if chainlink_update_ms > 0 && binance_price_now > 0.0 {
            now_ms - chainlink_update_ms
        } else {
            0
        };

        // At exact window boundaries (now_s % window_secs == 0) this second belongs
        // to the PREVIOUS window as its close tick (secs_left = 0).  All other seconds
        // count down from window_secs-1 to 1 within their window.
        let rem = now_s % window_secs;
        let (window_ts, window_secs_left) = if rem == 0 {
            (now_s - window_secs, 0)
        } else {
            (now_s - rem, window_secs - rem)
        };

        let tick = Tick {
            ts_ms: now_ms,
            yes_bid,
            yes_ask,
            no_bid,
            no_ask,
            yes_mid,
            binance_price: binance_price_now,
            chainlink_price: chainlink_price_now,
            oracle_lag_ms,
            window_ts,
            window_secs_left,
            ask_depth_usd: cached_ask_depth_usd,
            bid_depth_usd: cached_bid_depth_usd,
            window_yes_won: None, // live recorder: resolution not known until market settles
        };

        // ── Write JSONL ───────────────────────────────────────────────────────
        if let Some(ref mut f) = file {
            if let Ok(mut line) = serde_json::to_string(&tick) {
                line.push('\n');
                if let Err(e) = f.write_all(line.as_bytes()).await {
                    tracing::warn!("[TICK_RECORDER] Write error: {e}");
                }
            }
        }

        tracing::debug!(
            "[TICK_RECORDER] {} ts={} yes_mid={:.3} binance={:.2} cl={:.2} lag={}ms",
            cfg.slug, now_ms, yes_mid, binance_price_now, chainlink_price_now, oracle_lag_ms
        );
    }

    // Flush on exit
    if let Some(ref mut f) = file {
        let _ = f.flush().await;
    }
    Ok(())
}

// ── CLOB price fetcher ────────────────────────────────────────────────────────

/// Returns (yes_bid, yes_ask, no_bid, no_ask). All 0.0 on error.
async fn fetch_clob_prices(
    client: &reqwest::Client,
    base_url: &str,
    condition_id: &str,
) -> (f64, f64, f64, f64) {
    let url = format!("{}/book?token_id={}", base_url, condition_id);
    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
            match resp.json::<serde_json::Value>().await {
                Ok(json) => {
                    let yes_bid = best_price(&json, "bids");
                    let yes_ask = best_price(&json, "asks");
                    // NO token = complement of YES token in typical 2-token market.
                    // For simplicity use 1 - ask as NO bid, 1 - bid as NO ask.
                    let no_bid = if yes_ask > 0.0 { (1.0 - yes_ask).max(0.0) } else { 0.0 };
                    let no_ask = if yes_bid > 0.0 { (1.0 - yes_bid).max(0.0) } else { 0.0 };
                    (yes_bid, yes_ask, no_bid, no_ask)
                }
                Err(_) => (0.0, 0.0, 0.0, 0.0),
            }
        }
        _ => (0.0, 0.0, 0.0, 0.0),
    }
}

/// Fetch aggregated USD depth within 2% of best ask/bid from CLOB `/book`.
/// Returns (ask_depth_usd, bid_depth_usd). Both 0 on error.
async fn fetch_book_depth(
    http: &reqwest::Client,
    base_url: &str,
    condition_id: &str,
) -> (f64, f64) {
    let url = format!("{}/book?token_id={}", base_url.trim_end_matches('/'), condition_id);
    let body = match http.get(&url)
        .timeout(std::time::Duration::from_secs(5))
        .send().await
    {
        Ok(r) if r.status().is_success() => match r.json::<serde_json::Value>().await {
            Ok(v) => v,
            Err(_) => return (0.0, 0.0),
        },
        _ => return (0.0, 0.0),
    };

    fn side_depth(json: &serde_json::Value, key: &str, best_price: f64, pct: f64) -> f64 {
        let levels = match json.get(key).and_then(|v| v.as_array()) {
            Some(a) => a,
            None => return 0.0,
        };
        let threshold = best_price * (1.0 + pct);
        levels.iter().map(|level| {
            let price = level.get("price")
                .and_then(|v| v.as_str().and_then(|s| s.parse::<f64>().ok())
                    .or_else(|| v.as_f64()))
                .unwrap_or(0.0);
            let size = level.get("size")
                .and_then(|v| v.as_str().and_then(|s| s.parse::<f64>().ok())
                    .or_else(|| v.as_f64()))
                .unwrap_or(0.0);
            if price > 0.0 && price <= threshold { price * size } else { 0.0 }
        }).sum()
    }

    let best_ask = best_price(&body, "asks");
    let best_bid = best_price(&body, "bids");
    let ask_depth = side_depth(&body, "asks", best_ask, 0.02);
    let bid_depth = side_depth(&body, "bids", best_bid, 0.02);
    (ask_depth, bid_depth)
}

fn best_price(json: &serde_json::Value, side: &str) -> f64 {
    json.get(side)
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|entry| entry.get("price").or_else(|| entry.get("p")))
        .and_then(|p| match p {
            serde_json::Value::String(s) => s.parse().ok(),
            serde_json::Value::Number(n) => n.as_f64(),
            _ => None,
        })
        .unwrap_or(0.0)
}

// ── Chainlink price extractor (shared with live_feed.rs logic) ────────────────

fn extract_price(json: &serde_json::Value) -> Option<f64> {
    if let Some(p) = json.get("price").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()) { return Some(p); }
    if let Some(p) = json.get("price").and_then(|v| v.as_f64()) { return Some(p); }
    if let Some(p) = json.get("benchmarkPrice").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()) { return Some(p); }
    if let Some(p) = json.get("benchmarkPrice").and_then(|v| v.as_f64()) { return Some(p); }
    if let Some(p) = json.get("data").and_then(|d| d.get("price")).and_then(|v| v.as_str()).and_then(|s| s.parse().ok()) { return Some(p); }
    if let Some(p) = json.get("data").and_then(|d| d.get("price")).and_then(|v| v.as_f64()) { return Some(p); }
    None
}

// ── Old file pruner ───────────────────────────────────────────────────────────
//
// SAFETY: retain_days = 0 disables pruning entirely (keep ALL historical files).
// This is the recommended setting for slugs whose directory contains regenerated
// historical ticks from `to-ticks-multi` — pruning would delete that work.
async fn prune_old_files(dir: &Path, retain_days: u64) {
    if retain_days == 0 {
        // Pruning disabled — preserve all files (e.g. historical regenerated data)
        return;
    }
    let cutoff = chrono::Utc::now() - chrono::Duration::days(retain_days as i64);
    let cutoff_str = cutoff.format("%Y-%m-%d").to_string();
    let mut rd = match tokio::fs::read_dir(dir).await {
        Ok(r) => r,
        Err(_) => return,
    };
    while let Ok(Some(entry)) = rd.next_entry().await {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.ends_with(".jsonl") {
            let date_part = name_str.trim_end_matches(".jsonl");
            if date_part < cutoff_str.as_str() {
                let _ = tokio::fs::remove_file(entry.path()).await;
                tracing::info!("[TICK_RECORDER] Pruned old file: {}", entry.path().display());
            }
        }
    }
}

// ── Global registry ───────────────────────────────────────────────────────────
// A process-wide map of running recorders, keyed by slug.
// Allows the gateway API and tools to start/stop/query recorders by name.

use std::collections::HashMap;
use std::sync::OnceLock;
use tokio::sync::Mutex;

static RECORDERS: OnceLock<Mutex<HashMap<String, TickRecorderHandle>>> = OnceLock::new();

fn recorders() -> &'static Mutex<HashMap<String, TickRecorderHandle>> {
    RECORDERS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Start a recorder for `slug`, stopping any existing one first.
pub async fn start_recorder(cfg: TickRecorderConfig) {
    let mut map = recorders().lock().await;
    if let Some(old) = map.remove(&cfg.slug) {
        old.stop();
    }
    let slug = cfg.slug.clone();
    let handle = TickRecorder::start(cfg);
    map.insert(slug, handle);
}

/// Stop the recorder for `slug`. Returns true if one was running.
pub async fn stop_recorder(slug: &str) -> bool {
    let mut map = recorders().lock().await;
    if let Some(h) = map.remove(slug) {
        h.stop();
        true
    } else {
        false
    }
}

/// Returns slugs of all currently running recorders.
pub async fn running_recorders() -> Vec<String> {
    let map = recorders().lock().await;
    map.iter()
        .filter(|(_, h)| h.is_running())
        .map(|(k, _)| k.clone())
        .collect()
}

/// Read the last N ticks from the current day's JSONL file for a slug.
pub async fn read_recent_ticks(
    workspace_dir: &Path,
    slug: &str,
    last_n: usize,
) -> anyhow::Result<Vec<Tick>> {
    let day_str = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let path = workspace_dir
        .join("data")
        .join("ticks")
        .join(slug)
        .join(format!("{day_str}.jsonl"));

    let content = tokio::fs::read_to_string(&path).await?;
    let mut ticks: Vec<Tick> = content
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    if ticks.len() > last_n {
        ticks = ticks.split_off(ticks.len() - last_n);
    }
    Ok(ticks)
}

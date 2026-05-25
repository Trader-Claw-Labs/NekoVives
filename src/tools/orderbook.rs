//! Polymarket Orderbook Archive tool — wraps `tools/orderbook_parser.py`
//!
//! Provides gateway endpoints to:
//!   - Query remote Parquet files via DuckDB (no local storage needed)
//!   - Trigger a background download job for N days of hourly Parquet files
//!   - Poll download progress
//!   - List locally-downloaded files with size / date info

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

// ── Progress shared state ─────────────────────────────────────────────────────

/// Shared state for the background download / ingest job.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DownloadProgress {
    pub running: bool,
    pub phase: String,          // "downloading" | "converting" | "done" | ""
    pub done: usize,
    pub total: usize,
    pub current_hour: String,
    pub downloaded: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
    pub out_dir: String,
    pub slug: String,           // set by ingest jobs
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

/// Wrapper to hold all orderbook-related state.
pub struct OrderbookState {
    pub progress: Arc<Mutex<DownloadProgress>>,
    pub cancel: Arc<AtomicBool>,
    pub workspace_dir: PathBuf,
}

impl OrderbookState {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self {
            progress: Arc::new(Mutex::new(Default::default())),
            cancel: Arc::new(AtomicBool::new(false)),
            workspace_dir,
        }
    }

    /// Default download output directory.
    pub fn out_dir(&self) -> PathBuf {
        self.workspace_dir.join("data").join("orderbook")
    }

    /// Path to the parser script (relative to workspace root, two levels up from src/).
    pub fn parser_script(&self) -> PathBuf {
        // workspace_dir is ~/.traderclaw/workspace; script lives next to the binary's
        // source. We ship the script embedded and write it out on demand.
        self.workspace_dir
            .join("tools")
            .join("orderbook_parser.py")
    }
}

// ── Embedded script helper ────────────────────────────────────────────────────

/// The Python parser script, embedded at compile time.
const PARSER_SCRIPT: &str = include_str!(
    "../../tools/orderbook_parser.py"
);

/// Ensure the Python parser script exists in `<workspace>/tools/`.
/// Writes it if missing or if the content changed.
pub fn ensure_parser_script(workspace_dir: &Path) -> Result<PathBuf> {
    let dir = workspace_dir.join("tools");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("orderbook_parser.py");
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    if current != PARSER_SCRIPT {
        std::fs::write(&path, PARSER_SCRIPT)?;
        tracing::info!("[orderbook] wrote parser script to {}", path.display());
    }
    Ok(path)
}

// ── Query runners ─────────────────────────────────────────────────────────────

/// Request body for remote query endpoint.
#[derive(Debug, Deserialize)]
pub struct QueryRequest {
    /// Number of past days to query (1–30).
    pub days: u32,
    /// Analysis mode: "summary" | "price-series" | "top-markets" | "spread-stats" | "drift"
    pub mode: String,
    /// Optional condition ID (0x...). Required for price-series / spread-stats / drift.
    pub market: Option<String>,
    /// Candle frequency for price-series (pandas offset, e.g. "5min").
    pub freq: Option<String>,
    /// Drift window in seconds (default 300).
    pub window_secs: Option<u32>,
    /// For top-markets / summary: number of hours to sample (1-6). Default 1.
    /// Each file is 100-400 MB; keep low for remote queries.
    pub sample_hours: Option<u32>,
}

/// Download request body.
#[derive(Debug, Deserialize)]
pub struct DownloadRequest {
    pub days: u32,
    /// Optional condition ID — only download data for this market (much smaller files).
    pub market: Option<String>,
}

/// Ingest request body: download + auto-convert to ticks in one job.
#[derive(Debug, Deserialize)]
pub struct IngestRequest {
    pub days: u32,
    /// Polymarket condition ID (0x...) — required to filter data for one market.
    pub market: String,
    /// Short name for the tick slug (e.g. "btc_5m").
    pub slug: String,
    /// Binance symbol used as the underlying price feed (e.g. "BTCUSDT").
    pub binance_symbol: String,
}

/// Run the Python parser with the given sub-command + args.
/// Returns parsed JSON on success.
pub async fn run_parser(
    workspace_dir: &Path,
    sub_cmd: &str,
    extra_args: &[&str],
) -> Result<serde_json::Value> {
    let script = ensure_parser_script(workspace_dir)?;
    let mut cmd = tokio::process::Command::new("python3");
    cmd.arg(&script).arg(sub_cmd).args(extra_args);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let out = cmd.output().await.context("failed to spawn python3")?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !stderr.trim().is_empty() {
        tracing::debug!("[orderbook] parser stderr: {}", stderr.trim());
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let val: serde_json::Value = serde_json::from_str(stdout.trim())
        .with_context(|| format!("parser returned non-JSON: {}", stdout.trim()))?;
    Ok(val)
}

/// Spawn a background download task.
/// Progress is written to a temp JSON file polled by the Rust progress endpoint.
pub fn spawn_download(
    workspace_dir: PathBuf,
    days: u32,
    market: Option<String>,
    progress: Arc<Mutex<DownloadProgress>>,
    cancel: Arc<AtomicBool>,
) {
    tokio::spawn(async move {
        let out_dir = workspace_dir.join("data").join("orderbook");
        let progress_file = workspace_dir.join("data").join("orderbook_progress.json");

        {
            let mut p = progress.lock().await;
            *p = DownloadProgress {
                running: true,
                out_dir: out_dir.to_string_lossy().to_string(),
                started_at: Some(chrono::Utc::now().to_rfc3339()),
                ..Default::default()
            };
        }

        cancel.store(false, Ordering::SeqCst);

        let script = match ensure_parser_script(&workspace_dir) {
            Ok(s) => s,
            Err(e) => {
                let mut p = progress.lock().await;
                p.running = false;
                p.errors.push(format!("script error: {e}"));
                return;
            }
        };

        let days_str = days.to_string();
        let out_dir_str = out_dir.to_string_lossy().to_string();
        let progress_file_str = progress_file.to_string_lossy().to_string();
        let market_str = market.clone().unwrap_or_default();

        let mut extra_args: Vec<&str> = vec![
            "--days", &days_str,
            "--out", &out_dir_str,
            "--progress", &progress_file_str,
        ];
        if market.is_some() {
            extra_args.push("--market");
            extra_args.push(&market_str);
        }

        tracing::info!(
            "[orderbook] starting download: days={days} market={market:?} out={}",
            out_dir.display()
        );

        let child = tokio::process::Command::new("python3")
            .arg(&script)
            .arg("download")
            .args(&extra_args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                let mut p = progress.lock().await;
                p.running = false;
                p.finished_at = Some(chrono::Utc::now().to_rfc3339());
                p.errors.push(format!("spawn failed: {e}"));
                return;
            }
        };

        // Poll progress file every 5 seconds while the child runs
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;

            // Check cancel flag
            if cancel.load(Ordering::SeqCst) {
                let _ = child.kill().await;
                let mut p = progress.lock().await;
                p.running = false;
                p.finished_at = Some(chrono::Utc::now().to_rfc3339());
                p.errors.push("cancelled by user".to_string());
                return;
            }

            // Try reading progress file
            if let Ok(raw) = std::fs::read_to_string(&progress_file) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                    let mut p = progress.lock().await;
                    p.done = v["done"].as_u64().unwrap_or(0) as usize;
                    p.total = v["total"].as_u64().unwrap_or(0) as usize;
                    p.current_hour = v["current"].as_str().unwrap_or("").to_string();
                    p.downloaded = v["downloaded"].as_u64().unwrap_or(0) as usize;
                    p.skipped = v["skipped"].as_u64().unwrap_or(0) as usize;
                    if let Some(arr) = v["errors"].as_array() {
                        p.errors = arr.iter().filter_map(|e| e.as_str().map(String::from)).collect();
                    }
                }
            }

            // Check if child exited
            match child.try_wait() {
                Ok(Some(_status)) => break,
                Ok(None) => continue, // still running
                Err(e) => {
                    tracing::warn!("[orderbook] wait error: {e}");
                    break;
                }
            }
        }

        // Final state
        let mut p = progress.lock().await;
        p.running = false;
        p.finished_at = Some(chrono::Utc::now().to_rfc3339());
        tracing::info!(
            "[orderbook] download finished: downloaded={} skipped={} errors={}",
            p.downloaded,
            p.skipped,
            p.errors.len()
        );
    });
}

// ── Local file listing ────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct LocalFileInfo {
    pub filename: String,
    pub hour: String,
    pub size_mb: f64,
}

/// Spawn a combined download + to-ticks ingest job.
/// Phase 1: download hourly parquets for the given market/days.
/// Phase 2: run `to-ticks` conversion → writes JSONL to data/ticks/<slug>/.
pub fn spawn_ingest(
    workspace_dir: PathBuf,
    days: u32,
    market: String,
    slug: String,
    binance_symbol: String,
    progress: Arc<Mutex<DownloadProgress>>,
    cancel: Arc<AtomicBool>,
) {
    tokio::spawn(async move {
        let out_dir = workspace_dir.join("data").join("orderbook");
        let progress_file = workspace_dir.join("data").join("orderbook_progress.json");

        // ── Phase 1: download ──────────────────────────────────────────────────
        {
            let mut p = progress.lock().await;
            *p = DownloadProgress {
                running: true,
                phase: "downloading".to_string(),
                out_dir: out_dir.to_string_lossy().to_string(),
                slug: slug.clone(),
                started_at: Some(chrono::Utc::now().to_rfc3339()),
                ..Default::default()
            };
        }

        cancel.store(false, Ordering::SeqCst);

        let script = match ensure_parser_script(&workspace_dir) {
            Ok(s) => s,
            Err(e) => {
                let mut p = progress.lock().await;
                p.running = false;
                p.phase = "done".to_string();
                p.errors.push(format!("script error: {e}"));
                return;
            }
        };

        let days_str = days.to_string();
        let out_dir_dl = out_dir.to_string_lossy().to_string();
        let progress_file_str = progress_file.to_string_lossy().to_string();

        tracing::info!(
            "[orderbook/ingest] starting download: days={days} market={market} slug={slug}"
        );

        let child = tokio::process::Command::new("python3")
            .arg(&script)
            .arg("download")
            .args(["--days", &days_str,
                   "--out", &out_dir_dl,
                   "--market", &market,
                   "--progress", &progress_file_str])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                let mut p = progress.lock().await;
                p.running = false;
                p.phase = "done".to_string();
                p.finished_at = Some(chrono::Utc::now().to_rfc3339());
                p.errors.push(format!("spawn failed: {e}"));
                return;
            }
        };

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;

            if cancel.load(Ordering::SeqCst) {
                let _ = child.kill().await;
                let mut p = progress.lock().await;
                p.running = false;
                p.phase = "done".to_string();
                p.finished_at = Some(chrono::Utc::now().to_rfc3339());
                p.errors.push("cancelled by user".to_string());
                return;
            }

            if let Ok(raw) = std::fs::read_to_string(&progress_file) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                    let mut p = progress.lock().await;
                    p.done = v["done"].as_u64().unwrap_or(0) as usize;
                    p.total = v["total"].as_u64().unwrap_or(0) as usize;
                    p.current_hour = v["current"].as_str().unwrap_or("").to_string();
                    p.downloaded = v["downloaded"].as_u64().unwrap_or(0) as usize;
                    p.skipped = v["skipped"].as_u64().unwrap_or(0) as usize;
                    if let Some(arr) = v["errors"].as_array() {
                        p.errors = arr.iter().filter_map(|e| e.as_str().map(String::from)).collect();
                    }
                }
            }

            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => continue,
                Err(e) => { tracing::warn!("[orderbook/ingest] wait error: {e}"); break; }
            }
        }

        {
            let mut p = progress.lock().await;
            tracing::info!(
                "[orderbook/ingest] download done: downloaded={} skipped={} errors={}",
                p.downloaded, p.skipped, p.errors.len()
            );
        }

        // ── Phase 2: to-ticks conversion ───────────────────────────────────────
        {
            let mut p = progress.lock().await;
            p.phase = "converting".to_string();
            p.current_hour = format!("Converting → data/ticks/{slug}/");
        }

        let ticks_out = workspace_dir.join("data").join("ticks").join(&slug);
        let out_dir_str = out_dir.to_string_lossy().to_string();
        let ticks_out_str = ticks_out.to_string_lossy().to_string();

        tracing::info!("[orderbook/ingest] running to-ticks for slug={slug}");

        let result = tokio::process::Command::new("python3")
            .arg(&script)
            .arg("to-ticks")
            .args(["--market", &market,
                   "--slug", &slug,
                   "--binance-symbol", &binance_symbol,
                   "--in", &out_dir_str,
                   "--out", &ticks_out_str])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await;

        let mut p = progress.lock().await;
        p.running = false;
        p.phase = "done".to_string();
        p.finished_at = Some(chrono::Utc::now().to_rfc3339());

        match result {
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                if !out.status.success() {
                    p.errors.push(format!("to-ticks failed: {}", stderr.trim()));
                    tracing::error!("[orderbook/ingest] to-ticks failed: {}", stderr.trim());
                } else {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    tracing::info!("[orderbook/ingest] to-ticks done: {}", stdout.trim());
                }
            }
            Err(e) => {
                p.errors.push(format!("to-ticks spawn error: {e}"));
            }
        }
    });
}

/// List downloaded Parquet files in the orderbook data directory.
pub fn list_local_files(workspace_dir: &Path) -> Vec<LocalFileInfo> {
    let dir = workspace_dir.join("data").join("orderbook");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return vec![];
    };
    let mut files: Vec<LocalFileInfo> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.ends_with(".parquet") {
                return None;
            }
            let size_mb = e.metadata().map(|m| m.len() as f64 / 1_048_576.0).unwrap_or(0.0);
            let hour = name.trim_end_matches(".parquet").to_string();
            Some(LocalFileInfo { filename: name, hour, size_mb })
        })
        .collect();
    files.sort_by(|a, b| b.hour.cmp(&a.hour)); // newest first
    files
}

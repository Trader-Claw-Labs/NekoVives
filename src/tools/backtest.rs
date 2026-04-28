use super::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::path::PathBuf;

// ── Bundled default strategy scripts (embedded at compile time) ──────────────

const POLYMARKET_4MIN_SCRIPT: &str = include_str!("scripts/polymarket_4min.rhai");
const POLYMARKET_5MIN_SCRIPT: &str = include_str!("scripts/polymarket_5min.rhai");
const POLYMARKET_BTC_BINARY_SCRIPT: &str = include_str!("scripts/polymarket_btc_binary.rhai");
const CRYPTO_4MIN_SCRIPT: &str = include_str!("scripts/crypto_4min.rhai");
const STRATEGY_REFERENCE_SCRIPT: &str = include_str!("scripts/strategy_reference.rhai");
const STRATEGY_SCRIPT: &str = include_str!("scripts/strategy.rhai");

const WEATHER_BINARY_SCRIPT: &str = include_str!("scripts/weather_binary.rhai");

// ── 8 Advanced Strategy Scripts ──────────────────────────────────────────────
const MEAN_REVERSION_SCRIPT: &str = include_str!("scripts/mean_reversion.rhai");
const DCA_BOT_SCRIPT: &str = include_str!("scripts/dca_bot.rhai");
const PUMP_DETECTION_SCRIPT: &str = include_str!("scripts/pump_detection.rhai");
const GRID_TRADING_SCRIPT: &str = include_str!("scripts/grid_trading.rhai");
const SPREAD_ARB_SCRIPT: &str = include_str!("scripts/spread_arb.rhai");
const EVENT_DRIVEN_SCRIPT: &str = include_str!("scripts/event_driven.rhai");
const CORRELATION_ARB_SCRIPT: &str = include_str!("scripts/correlation_arb.rhai");
const LIQUIDATION_HUNT_SCRIPT: &str = include_str!("scripts/liquidation_hunt.rhai");

/// Write bundled default scripts to `<workspace>/scripts/` if they don't exist yet.
/// Called by both backtest tools so the scripts are always available on first run.
/// All bundled default scripts as (filename, content) pairs.
const DEFAULT_SCRIPTS: [(&str, &str); 15] = [
    ("polymarket_4min.rhai",        POLYMARKET_4MIN_SCRIPT),
    ("polymarket_5min.rhai",        POLYMARKET_5MIN_SCRIPT),
    ("polymarket_btc_binary.rhai",  POLYMARKET_BTC_BINARY_SCRIPT),
    ("weather_binary.rhai",         WEATHER_BINARY_SCRIPT),
    ("crypto_4min.rhai",            CRYPTO_4MIN_SCRIPT),
    ("strategy_reference.rhai",     STRATEGY_REFERENCE_SCRIPT),
    ("strategy.rhai",               STRATEGY_SCRIPT),
    ("mean_reversion.rhai",         MEAN_REVERSION_SCRIPT),
    ("dca_bot.rhai",                DCA_BOT_SCRIPT),
    ("pump_detection.rhai",         PUMP_DETECTION_SCRIPT),
    ("grid_trading.rhai",           GRID_TRADING_SCRIPT),
    ("spread_arb.rhai",             SPREAD_ARB_SCRIPT),
    ("event_driven.rhai",           EVENT_DRIVEN_SCRIPT),
    ("correlation_arb.rhai",        CORRELATION_ARB_SCRIPT),
    ("liquidation_hunt.rhai",       LIQUIDATION_HUNT_SCRIPT),
];

pub fn ensure_default_scripts(workspace_dir: &std::path::Path) {
    let scripts_dir = workspace_dir.join("scripts");
    let _ = std::fs::create_dir_all(&scripts_dir);
    for (name, content) in &DEFAULT_SCRIPTS {
        let path = scripts_dir.join(name);
        if !path.exists() {
            let _ = std::fs::write(&path, content);
        }
    }
}

/// Read a script from the workspace, or fall back to the bundled default.
/// If the file does not exist but matches a bundled default name, writes it
/// to disk first so the user can inspect/edit it later.
pub fn read_script_or_default(workspace_dir: &std::path::Path, name: &str) -> Option<String> {
    let scripts_dir = workspace_dir.join("scripts");
    let path = scripts_dir.join(name);
    if path.exists() {
        return std::fs::read_to_string(&path).ok();
    }
    // Fallback: write bundled default to disk and return it
    for (default_name, content) in &DEFAULT_SCRIPTS {
        if *default_name == name {
            let _ = std::fs::create_dir_all(&scripts_dir);
            let _ = std::fs::write(&path, content);
            return Some(content.to_string());
        }
    }
    None
}

// ── backtest_list_scripts ────────────────────────────────────────────

/// List available .rhai strategy scripts the agent can run backtests on.
pub struct BacktestListScriptsTool {
    workspace_dir: PathBuf,
}

impl BacktestListScriptsTool {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }
}

#[async_trait]
impl Tool for BacktestListScriptsTool {
    fn name(&self) -> &str {
        "backtest_list_scripts"
    }

    fn description(&self) -> &str {
        "List all .rhai trading strategy scripts available for backtesting. \
        Returns the filename, full path, and first-line description of each script."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        ensure_default_scripts(&self.workspace_dir);
        let scripts_dir = self.workspace_dir.join("scripts");
        let _ = std::fs::create_dir_all(&scripts_dir);

        let mut scripts = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&scripts_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("rhai") {
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string();
                    let description = std::fs::read_to_string(&path)
                        .ok()
                        .and_then(|c| {
                            c.lines()
                                .next()
                                .map(|l| l.trim_start_matches("//").trim().to_string())
                        })
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "(no description)".to_string());
                    scripts.push(format!(
                        "- {} — {} (path: {})",
                        name,
                        description,
                        path.display()
                    ));
                }
            }
        }

        if scripts.is_empty() {
            return Ok(ToolResult {
                success: true,
                output: format!(
                    "No .rhai scripts found in {}. \
                    Use the file_write tool to create one first.",
                    scripts_dir.display()
                ),
                error: None,
            });
        }

        Ok(ToolResult {
            success: true,
            output: format!(
                "Found {} script(s) in {}:\n{}",
                scripts.len(),
                scripts_dir.display(),
                scripts.join("\n")
            ),
            error: None,
        })
    }
}

// ── backtest_run ─────────────────────────────────────────────────────

/// Run a backtest on a .rhai strategy script and return performance metrics.
pub struct BacktestRunTool {
    workspace_dir: PathBuf,
}

impl BacktestRunTool {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }
}

#[async_trait]
impl Tool for BacktestRunTool {
    fn name(&self) -> &str {
        "backtest_run"
    }

    fn description(&self) -> &str {
        "Run a backtest on a .rhai trading strategy script and return performance metrics \
        including total return, Sharpe ratio, max drawdown, win rate, trade count, \
        and the 5 worst trades. The script must exist in the scripts/ directory."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "script": {
                    "type": "string",
                    "description": "Filename of the .rhai script (e.g. 'rsi_btc.rhai') or full path"
                },
                "market_type": {
                    "type": "string",
                    "description": "Market type: 'crypto' for Binance data or 'polymarket' for prediction markets",
                    "enum": ["crypto", "polymarket"],
                    "default": "crypto"
                },
                "symbol": {
                    "type": "string",
                    "description": "For crypto: trading pair (e.g. 'BTCUSDT'). For polymarket: condition token ID",
                    "default": "BTCUSDT"
                },
                "from_date": {
                    "type": "string",
                    "description": "Start date for backtest in YYYY-MM-DD format",
                    "default": "2024-01-01"
                },
                "to_date": {
                    "type": "string",
                    "description": "End date for backtest in YYYY-MM-DD format",
                    "default": "2024-12-31"
                },
                "initial_balance": {
                    "type": "number",
                    "description": "Starting portfolio balance in USD",
                    "default": 10000.0
                },
                "fee_pct": {
                    "type": "number",
                    "description": "Trading fee percentage per trade (e.g. 0.1 for 0.1%)",
                    "default": 0.1
                },
                "interval": {
                    "type": "string",
                    "description": "Candle interval/timeframe (e.g. '1m', '5m', '15m', '1h', '4h', '1d')",
                    "default": "1m"
                }
            },
            "required": ["script"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let script_input = args
            .get("script")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'script' parameter"))?;

        let market_type = args
            .get("market_type")
            .and_then(|v| v.as_str())
            .unwrap_or("crypto");
        let symbol = args
            .get("symbol")
            .and_then(|v| v.as_str())
            .unwrap_or("BTCUSDT");
        let from_date = args
            .get("from_date")
            .and_then(|v| v.as_str())
            .unwrap_or("2024-01-01");
        let to_date = args
            .get("to_date")
            .and_then(|v| v.as_str())
            .unwrap_or("2024-12-31");
        let initial_balance = args
            .get("initial_balance")
            .and_then(|v| v.as_f64())
            .unwrap_or(10_000.0);
        let fee_pct = args
            .get("fee_pct")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.1);
        let interval = args
            .get("interval")
            .and_then(|v| v.as_str())
            .unwrap_or("1m");

        ensure_default_scripts(&self.workspace_dir);

        // Resolve path: if input is just a filename, look in scripts/
        let script_path = {
            let p = std::path::Path::new(script_input);
            if p.is_absolute() || p.components().count() > 1 {
                p.to_path_buf()
            } else {
                self.workspace_dir.join("scripts").join(script_input)
            }
        };

        if !script_path.exists() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "Script not found: {}. Use backtest_list_scripts to see available scripts.",
                    script_path.display()
                )),
            });
        }

        // Run the real Rhai backtest engine
        let metrics = run_backtest_engine(
            &script_path, market_type, symbol, interval, from_date, to_date,
            initial_balance, fee_pct, "price_up", None, None, None, "percent", 1.0, &self.workspace_dir
        ).await;

        let worst_trades_text = metrics
            .worst_trades
            .iter()
            .enumerate()
            .map(|(i, t)| {
                format!(
                    "  {}. {} {} @ ${:.2} — PnL: ${:.2}",
                    i + 1,
                    t.side,
                    t.timestamp,
                    t.price,
                    t.pnl
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let output = format!(
            "Backtest Results — {} on {} ({} to {})\n\
            Initial balance: ${:.2} | Fee: {:.2}%\n\
            ─────────────────────────────────────\n\
            Total Return:   {:.2}%\n\
            Sharpe Ratio:   {:.2}\n\
            Max Drawdown:   {:.2}%\n\
            Win Rate:       {:.1}%\n\
            Total Trades:   {}\n\
            ─────────────────────────────────────\n\
            5 Worst Trades:\n\
            {}\n\
            ─────────────────────────────────────\n\
            Analysis:\n{}",
            script_path.file_name().unwrap_or_default().to_string_lossy(),
            symbol,
            from_date,
            to_date,
            initial_balance,
            fee_pct,
            metrics.total_return_pct,
            metrics.sharpe_ratio,
            metrics.max_drawdown_pct,
            metrics.win_rate_pct,
            metrics.total_trades,
            worst_trades_text,
            metrics.analysis,
        );

        Ok(ToolResult {
            success: true,
            output,
            error: None,
        })
    }
}

// ── Internal engine ──────────────────────────────────────────────────

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct BacktestMetrics {
    pub total_return_pct: f64,
    pub sharpe_ratio: f64,
    pub max_drawdown_pct: f64,
    pub win_rate_pct: f64,
    pub total_trades: u32,
    pub worst_trades: Vec<WorstTrade>,
    pub all_trades: Vec<AllTrade>,
    pub analysis: String,
    // Binary-market specific metrics (None for crypto backtests)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_token_price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correct_direction_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub break_even_win_rate: Option<f64>,
    // Number of Polymarket market windows tested (slug-aligned binary mode only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub markets_tested: Option<u32>,
    /// Debug values captured for flat (no-trade) windows, keyed by timestamp.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flat_debugs: Vec<(String, std::collections::HashMap<String, f64>)>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct WorstTrade {
    pub timestamp: String,
    pub side: String,
    pub price: f64,
    pub pnl: f64,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct AllTrade {
    pub timestamp: String,
    pub side: String,
    pub price: f64,
    pub size: f64,
    pub pnl: f64,
    pub balance: f64,  // portfolio balance after this trade
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug: Option<std::collections::HashMap<String, f64>>,
}

// ── Candle from Binance ──────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct Candle {
    pub open_time_ms: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// Fetch OHLCV candles from Binance REST API with pagination.
/// Automatically fetches multiple batches to cover the full date range.
/// Caches the result to `<workspace>/data/<symbol>_<interval>_<from>_<to>.json`.
async fn fetch_candles(
    symbol: &str,
    interval: &str,
    from_date: &str,
    to_date: &str,
    workspace_dir: &std::path::Path,
) -> anyhow::Result<Vec<Candle>> {
    use chrono::NaiveDate;

    // Convert dates to unix ms timestamps
    let from_ms = NaiveDate::parse_from_str(from_date, "%Y-%m-%d")
        .map_err(|e| anyhow::anyhow!("Invalid from_date: {e}"))?
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis();
    let to_ms = NaiveDate::parse_from_str(to_date, "%Y-%m-%d")
        .map_err(|e| anyhow::anyhow!("Invalid to_date: {e}"))?
        .and_hms_opt(23, 59, 59)
        .unwrap()
        .and_utc()
        .timestamp_millis();

    // Check cache
    let data_dir = workspace_dir.join("data");
    let _ = std::fs::create_dir_all(&data_dir);
    let cache_file = data_dir.join(format!("{}_{}_{from_date}_{to_date}.json", symbol.to_uppercase(), interval));
    if let Ok(cached) = std::fs::read_to_string(&cache_file) {
        if let Ok(candles) = serde_json::from_str::<Vec<CandleCache>>(&cached) {
            if !candles.is_empty() {
                tracing::info!("[BACKTEST] Loaded {} candles from cache", candles.len());
                return Ok(candles.into_iter().map(|c| Candle {
                    open_time_ms: c.open_time_ms,
                    open: c.open,
                    high: c.high,
                    low: c.low,
                    close: c.close,
                    volume: c.volume,
                }).collect());
            }
        }
    }

    // Fetch from Binance with pagination (1000 candles per request)
    let client = reqwest::Client::new();
    let mut all_candles: Vec<Candle> = Vec::new();
    let mut current_start = from_ms;
    let max_requests = 500; // Safety limit: 500 * 1000 = 500k candles max
    let mut request_count = 0;

    tracing::info!("[BACKTEST] Fetching {} candles from Binance (paginated)...", interval);

    while current_start < to_ms && request_count < max_requests {
        let url = format!(
            "https://api.binance.com/api/v3/klines?symbol={}&interval={}&startTime={}&endTime={}&limit=1000",
            symbol.to_uppercase(),
            interval,
            current_start,
            to_ms
        );

        let body = client
            .get(&url)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Binance request failed: {e}"))?
            .error_for_status()
            .map_err(|e| anyhow::anyhow!("Binance returned error: {e}"))?
            .text()
            .await
            .map_err(|e| anyhow::anyhow!("Binance response error: {e}"))?;

        let batch = parse_binance_klines(&body)?;
        let batch_len = batch.len();

        if batch.is_empty() {
            break;
        }

        // Update start time for next batch (use last candle's open_time + 1ms)
        if let Some(last) = batch.last() {
            current_start = last.open_time_ms + 1;
        }

        all_candles.extend(batch);
        request_count += 1;

        tracing::debug!(
            "[BACKTEST] Fetched batch {}: {} candles (total: {})",
            request_count, batch_len, all_candles.len()
        );

        // If we got less than 1000, we've reached the end
        if batch_len < 1000 {
            break;
        }

        // Small delay to avoid rate limiting
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    tracing::info!("[BACKTEST] Fetched {} total candles in {} requests", all_candles.len(), request_count);

    // Cache for next run (using a serializable format)
    let cache_data: Vec<CandleCache> = all_candles.iter().map(|c| CandleCache {
        open_time_ms: c.open_time_ms,
        open: c.open,
        high: c.high,
        low: c.low,
        close: c.close,
        volume: c.volume,
    }).collect();
    if let Ok(json) = serde_json::to_string(&cache_data) {
        let _ = std::fs::write(&cache_file, json);
    }

    Ok(all_candles)
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CandleCache {
    open_time_ms: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
}

// u2500u2500 Open-Meteo weather data fetcher u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500u2500
//
// Returns daily max-temperature data as synthetic Candle structs:
//   open  = previous day's max temp (for momentum calculation)
//   close = this day's max temp (the resolution value)
//   high  = this day's max temp
//   low   = this day's min temp
//   volume = 1.0 (no meaningful volume for weather)
//
// Timestamps are midnight UTC in milliseconds.
async fn fetch_weather_candles(
    city: &str,
    from_date: &str,
    to_date: &str,
) -> anyhow::Result<Vec<Candle>> {
    let (lat, lon) = crate::tools::series::city_coords(city)
        .ok_or_else(|| anyhow::anyhow!("Unknown city '{city}' u2014 supported: munich, london, nyc, paris, berlin, madrid, tokyo, chicago, sydney"))?;

    let url = format!(
        "https://archive-api.open-meteo.com/v1/archive\
        ?latitude={lat}&longitude={lon}\
        &start_date={from_date}&end_date={to_date}\
        &daily=temperature_2m_max,temperature_2m_min\
        &timezone=UTC",
    );

    let client = reqwest::Client::new();
    let body = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;

    #[derive(serde::Deserialize)]
    struct OmDaily {
        time: Vec<String>,
        temperature_2m_max: Vec<Option<f64>>,
        temperature_2m_min: Vec<Option<f64>>,
    }
    #[derive(serde::Deserialize)]
    struct OmResponse { daily: OmDaily }

    let resp: OmResponse = serde_json::from_str(&body)
        .map_err(|e| anyhow::anyhow!("Open-Meteo parse error: {e}\nBody: {}", &body[..body.len().min(300)]))?;

    let days = resp.daily.time.len();
    let mut candles: Vec<Candle> = Vec::with_capacity(days);
    let mut prev_max: f64 = 0.0;

    for i in 0..days {
        let date_str = &resp.daily.time[i];
        let max_temp = resp.daily.temperature_2m_max[i].unwrap_or(0.0);
        let min_temp = resp.daily.temperature_2m_min[i].unwrap_or(0.0);

        // Parse date to midnight-UTC timestamp in ms
        let ts_ms = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
            .map(|d| d.and_hms_opt(0, 0, 0).unwrap())
            .map(|dt| dt.and_utc().timestamp_millis())
            .unwrap_or(0);

        candles.push(Candle {
            open_time_ms: ts_ms,
            open:  if i == 0 { max_temp } else { prev_max },
            high:  max_temp,
            low:   min_temp,
            close: max_temp,
            volume: 1.0,
        });
        prev_max = max_temp;
    }

    Ok(candles)
}

fn parse_binance_klines(body: &str) -> anyhow::Result<Vec<Candle>> {
    let raw: Vec<Vec<serde_json::Value>> = serde_json::from_str(body)
        .map_err(|e| anyhow::anyhow!("Failed to parse Binance klines: {e}"))?;
    let candles = raw
        .into_iter()
        .filter_map(|row| {
            if row.len() < 6 { return None; }
            let open_time = row[0].as_i64()?;
            let open  = row[1].as_str()?.parse::<f64>().ok()?;
            let high  = row[2].as_str()?.parse::<f64>().ok()?;
            let low   = row[3].as_str()?.parse::<f64>().ok()?;
            let close = row[4].as_str()?.parse::<f64>().ok()?;
            let vol   = row[5].as_str()?.parse::<f64>().ok()?;
            Some(Candle { open_time_ms: open_time, open, high, low, close, volume: vol })
        })
        .collect();
    Ok(candles)
}

// ── Polymarket Data Fetching ─────────────────────────────────────────

/// Fetch price history from Polymarket CLOB API.
/// Returns candles with price as close (open=high=low=close since it's price history).
/// Caches the result to `<workspace>/data/poly_<token_id>_<interval>_<from>_<to>.json`.
async fn fetch_polymarket_candles(
    condition_id: &str,
    interval: &str,
    from_date: &str,
    to_date: &str,
    workspace_dir: &std::path::Path,
) -> anyhow::Result<Vec<Candle>> {
    use chrono::NaiveDate;

    // Auto-resolve slug → token_id if symbol doesn't look like a long numeric/hex ID.
    // The Polymarket CLOB /prices-history endpoint uses the binary token ID (a long integer
    // string like "71321045679252212594626385532706912750332728571942532289631379312455583992".
    // Event slugs (e.g. "btc-updown-5m-1776214500") need to be resolved via:
    //   1. Gamma events API  → event → markets → clobTokenIds[0]
    //   2. Gamma markets API → market → clobTokenIds[0]
    let resolved_id: String = {
        let looks_like_token_id = condition_id.chars().all(|c| c.is_ascii_digit()) && condition_id.len() > 20;
        let looks_like_hex_id = condition_id.starts_with("0x") && condition_id.len() > 20;
        if looks_like_token_id || looks_like_hex_id {
            condition_id.to_string()
        } else {
            let client = reqwest::Client::new();

            // Helper: extract first clobTokenId from a market JSON object
            fn first_clob_token(market: &serde_json::Value) -> Option<String> {
                // clobTokenIds is sometimes a JSON array, sometimes a JSON-encoded string
                if let Some(ids) = market.get("clobTokenIds") {
                    if let Some(arr) = ids.as_array() {
                        if let Some(id) = arr.first().and_then(|v| v.as_str()) {
                            return Some(id.to_string());
                        }
                    }
                    if let Some(s) = ids.as_str() {
                        // Might be a JSON string like "[\"123...\",\"456...\"]"
                        if let Ok(arr) = serde_json::from_str::<Vec<String>>(s) {
                            if let Some(id) = arr.into_iter().next() {
                                return Some(id);
                            }
                        }
                    }
                }
                // Fallback: conditionId
                market.get("conditionId").and_then(|v| v.as_str()).map(|s| s.to_string())
            }

            let mut resolved = condition_id.to_string();

            // 1. Try as event slug
            let event_url = format!("https://gamma-api.polymarket.com/events?slug={}", condition_id);
            if let Ok(resp) = client.get(&event_url).timeout(std::time::Duration::from_secs(10)).send().await {
                if resp.status().is_success() {
                    if let Ok(data) = resp.json::<serde_json::Value>().await {
                        // events endpoint returns an array; pick first event's first market
                        let token = data.as_array()
                            .and_then(|arr| arr.first())
                            .and_then(|ev| ev.get("markets"))
                            .and_then(|ms| ms.as_array())
                            .and_then(|ms| ms.first())
                            .and_then(|m| first_clob_token(m));
                        if let Some(t) = token {
                            tracing::info!("[BACKTEST] Resolved event slug '{}' → token_id {}", condition_id, &t[..t.len().min(20)]);
                            resolved = t;
                        }
                    }
                }
            }

            // 2. If still looks like a slug (not numeric), try market slug
            if !resolved.chars().all(|c| c.is_ascii_digit()) || resolved.len() < 20 {
                let market_url = format!("https://gamma-api.polymarket.com/markets?slug={}", condition_id);
                if let Ok(resp) = client.get(&market_url).timeout(std::time::Duration::from_secs(10)).send().await {
                    if resp.status().is_success() {
                        if let Ok(data) = resp.json::<serde_json::Value>().await {
                            let token = data.as_array()
                                .and_then(|arr| arr.first())
                                .and_then(|m| first_clob_token(m));
                            if let Some(t) = token {
                                tracing::info!("[BACKTEST] Resolved market slug '{}' → token_id {}", condition_id, &t[..t.len().min(20)]);
                                resolved = t;
                            }
                        }
                    }
                }
            }

            resolved
        }
    };
    let condition_id = resolved_id.as_str();

    // Convert dates to unix timestamps (seconds)
    let from_ts = NaiveDate::parse_from_str(from_date, "%Y-%m-%d")
        .map_err(|e| anyhow::anyhow!("Invalid from_date: {e}"))?
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp();
    let to_ts = NaiveDate::parse_from_str(to_date, "%Y-%m-%d")
        .map_err(|e| anyhow::anyhow!("Invalid to_date: {e}"))?
        .and_hms_opt(23, 59, 59)
        .unwrap()
        .and_utc()
        .timestamp();

    // Map interval to Polymarket fidelity (in minutes)
    let fidelity = match interval {
        "1m" => 1,
        "5m" => 5,
        "15m" => 15,
        "1h" => 60,
        "4h" => 240,
        "1d" => 1440,
        _ => 60, // Default to 1h
    };

    // Check cache
    let data_dir = workspace_dir.join("data");
    let _ = std::fs::create_dir_all(&data_dir);
    // Use first 16 chars of condition_id for filename
    let id_short = &condition_id[..std::cmp::min(16, condition_id.len())];
    let cache_file = data_dir.join(format!("poly_{}_{}_{from_date}_{to_date}.json", id_short, interval));

    if let Ok(cached) = std::fs::read_to_string(&cache_file) {
        if let Ok(candles) = serde_json::from_str::<Vec<CandleCache>>(&cached) {
            if !candles.is_empty() {
                tracing::info!("[BACKTEST] Loaded {} Polymarket candles from cache", candles.len());
                return Ok(candles.into_iter().map(|c| Candle {
                    open_time_ms: c.open_time_ms,
                    open: c.open,
                    high: c.high,
                    low: c.low,
                    close: c.close,
                    volume: c.volume,
                }).collect());
            }
        }
    }

    // Fetch from Polymarket CLOB API in chunks.
    //
    // The API rejects requests where (endTs - startTs) / fidelity_seconds > ~5000 points.
    // We split the range into chunks of at most MAX_POINTS_PER_CHUNK candles each and
    // concatenate the results.  The `interval` named-window param is omitted because we
    // provide explicit startTs/endTs.
    const MAX_POINTS_PER_CHUNK: i64 = 3_000;
    let fidelity_secs = (fidelity as i64) * 60;
    let chunk_secs = fidelity_secs * MAX_POINTS_PER_CHUNK;

    let client = reqwest::Client::new();
    let mut all_candles: Vec<Candle> = Vec::new();
    let mut chunk_start = from_ts;

    while chunk_start < to_ts {
        let chunk_end = (chunk_start + chunk_secs).min(to_ts);

        let url = format!(
            "https://clob.polymarket.com/prices-history?market={}&fidelity={}&startTs={}&endTs={}",
            condition_id, fidelity, chunk_start, chunk_end
        );
        tracing::info!("[BACKTEST] Fetching Polymarket chunk {} → {} (fidelity={}m)", chunk_start, chunk_end, fidelity);

        let response = client
            .get(&url)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Polymarket request failed: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Polymarket API error ({}): {}", status, body));
        }

        let body = response.text().await
            .map_err(|e| anyhow::anyhow!("Polymarket response error: {e}"))?;

        let chunk_candles = parse_polymarket_prices(&body)?;
        tracing::info!("[BACKTEST] Chunk returned {} price points", chunk_candles.len());
        all_candles.extend(chunk_candles);

        chunk_start = chunk_end + 1;
    }

    // Deduplicate by timestamp (overlaps possible at chunk boundaries) and sort
    all_candles.sort_by_key(|c| c.open_time_ms);
    all_candles.dedup_by_key(|c| c.open_time_ms);

    let candles = all_candles;
    tracing::info!("[BACKTEST] Fetched {} Polymarket price points total", candles.len());

    // Cache for next run
    let cache_data: Vec<CandleCache> = candles.iter().map(|c| CandleCache {
        open_time_ms: c.open_time_ms,
        open: c.open,
        high: c.high,
        low: c.low,
        close: c.close,
        volume: c.volume,
    }).collect();
    if let Ok(json) = serde_json::to_string(&cache_data) {
        let _ = std::fs::write(&cache_file, json);
    }

    Ok(candles)
}

fn parse_polymarket_prices(body: &str) -> anyhow::Result<Vec<Candle>> {
    #[derive(serde::Deserialize)]
    struct PricePoint {
        t: i64,  // timestamp in seconds
        p: f64,  // price (0.0 to 1.0)
    }

    #[derive(serde::Deserialize)]
    struct PriceHistory {
        history: Vec<PricePoint>,
    }

    let data: PriceHistory = serde_json::from_str(body)
        .map_err(|e| anyhow::anyhow!("Failed to parse Polymarket prices: {e}"))?;

    // Convert price points to candles
    // Group consecutive points and create OHLC candles
    let mut candles: Vec<Candle> = Vec::new();

    for point in data.history {
        // Price is between 0 and 1, multiply by 100 for percentage display
        let price = point.p * 100.0;
        candles.push(Candle {
            open_time_ms: point.t * 1000, // Convert to milliseconds
            open: price,
            high: price,
            low: price,
            close: price,
            volume: 1.0, // Polymarket doesn't provide volume in price history
        });
    }

    // Sort by timestamp
    candles.sort_by_key(|c| c.open_time_ms);

    Ok(candles)
}

// ── Technical indicators ─────────────────────────────────────────────

fn compute_ema(values: &[f64], period: usize) -> Vec<f64> {
    if values.len() < period { return vec![0.0; values.len()]; }
    let k = 2.0 / (period as f64 + 1.0);
    let mut ema = vec![0.0; values.len()];
    // Seed with simple MA of first `period` values
    ema[period - 1] = values[..period].iter().sum::<f64>() / period as f64;
    for i in period..values.len() {
        ema[i] = values[i] * k + ema[i - 1] * (1.0 - k);
    }
    ema
}

fn compute_rsi_series(closes: &[f64], period: usize) -> Vec<f64> {
    let mut rsi = vec![50.0_f64; closes.len()];
    if closes.len() < period + 1 { return rsi; }
    let mut avg_gain = 0.0_f64;
    let mut avg_loss = 0.0_f64;
    for i in 1..=period {
        let delta = closes[i] - closes[i - 1];
        if delta > 0.0 { avg_gain += delta; } else { avg_loss += delta.abs(); }
    }
    avg_gain /= period as f64;
    avg_loss /= period as f64;
    rsi[period] = if avg_loss == 0.0 { 100.0 } else { 100.0 - 100.0 / (1.0 + avg_gain / avg_loss) };
    for i in (period + 1)..closes.len() {
        let delta = closes[i] - closes[i - 1];
        let gain = if delta > 0.0 { delta } else { 0.0 };
        let loss = if delta < 0.0 { delta.abs() } else { 0.0 };
        avg_gain = (avg_gain * (period - 1) as f64 + gain) / period as f64;
        avg_loss = (avg_loss * (period - 1) as f64 + loss) / period as f64;
        rsi[i] = if avg_loss == 0.0 { 100.0 } else { 100.0 - 100.0 / (1.0 + avg_gain / avg_loss) };
    }
    rsi
}

fn compute_macd_series(closes: &[f64]) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let ema12 = compute_ema(closes, 12);
    let ema26 = compute_ema(closes, 26);
    let macd: Vec<f64> = ema12.iter().zip(ema26.iter()).map(|(a, b)| a - b).collect();
    let signal = compute_ema(&macd, 9);
    let histogram: Vec<f64> = macd.iter().zip(signal.iter()).map(|(m, s)| m - s).collect();
    (macd, signal, histogram)
}

/// Compute MACD histogram value at a specific index using configurable periods.
fn macd_hist_value(closes: &[f64], i: usize, fast: usize, slow: usize, signal: usize) -> f64 {
    if i == 0 || fast == 0 || slow == 0 || signal == 0 || fast >= slow || closes.len() <= i {
        return 0.0;
    }
    let fast_ema = compute_ema(&closes[..=i], fast);
    let slow_ema = compute_ema(&closes[..=i], slow);
    let mut macd_line = vec![0.0; i + 1];
    for j in 0..=i {
        macd_line[j] = fast_ema[j] - slow_ema[j];
    }
    let signal_line = compute_ema(&macd_line, signal);
    macd_line[i] - signal_line[i]
}

fn compute_atr_series(highs: &[f64], lows: &[f64], closes: &[f64], period: usize) -> Vec<f64> {
    let n = closes.len();
    if n == 0 { return Vec::new(); }
    let mut tr = vec![0.0_f64; n];
    tr[0] = highs[0] - lows[0];
    for i in 1..n {
        let hl = highs[i] - lows[i];
        let hc = (highs[i] - closes[i - 1]).abs();
        let lc = (lows[i]  - closes[i - 1]).abs();
        tr[i] = hl.max(hc).max(lc);
    }
    // ATR = smoothed moving average of TR
    let mut atr = vec![0.0_f64; n];
    if n < period { return atr; }
    atr[period - 1] = tr[..period].iter().sum::<f64>() / period as f64;
    for i in period..n {
        atr[i] = (atr[i - 1] * (period - 1) as f64 + tr[i]) / period as f64;
    }
    atr
}

// ── Rhai execution ───────────────────────────────────────────────────

#[derive(Clone, Default)]
struct Trade {
    timestamp: String,
    side: String,
    price: f64,
    size: f64, // base token amount
    pnl: f64,
    debug: Option<std::collections::HashMap<String, f64>>,
}

/// Run the Rhai script against all candles, simulate trades, return metrics.
///
/// Supports the `on_candle(ctx)` API where `ctx` is a dynamic object with:
///   ctx.close / ctx.open / ctx.high / ctx.low / ctx.volume / ctx.index
///   ctx.position / ctx.entry_price / ctx.entry_index / ctx.balance
///   ctx.close_at(i) / ctx.volume_at(i)
///   ctx.rsi(period) / ctx.ema(period) / ctx.atr(period)
///   ctx.buy(size) / ctx.sell(size)
///   ctx.set(key, val) / ctx.get(key, default)
///
/// Also supports legacy scripts that set a `signal` variable to "buy"/"sell"/"hold".
fn run_rhai_backtest(
    script_content: String,
    candles: Vec<Candle>,
    initial_balance: f64,
    fee_pct: f64,
    max_entry_price: Option<f64>,
    sizing_mode: &str,
    sizing_value: f64,
) -> anyhow::Result<BacktestMetrics> {
    use rhai::{Dynamic, Engine, Map, Scope};
    use std::sync::{Arc, Mutex};

    tracing::debug!("[BACKTEST-RHAI] Starting Rhai execution with {} candles", candles.len());

    let closes:  Vec<f64> = candles.iter().map(|c| c.close).collect();
    let highs:   Vec<f64> = candles.iter().map(|c| c.high).collect();
    let lows:    Vec<f64> = candles.iter().map(|c| c.low).collect();
    let volumes: Vec<f64> = candles.iter().map(|c| c.volume).collect();

    // Pre-compute ATR(14) series (used by many strategies)
    let atr14 = compute_atr_series(&highs, &lows, &closes, 14);

    let mut engine = Engine::new();
    engine.set_max_operations(500_000); // safety cap per candle
    engine.set_max_call_levels(64);

    tracing::debug!("[BACKTEST-RHAI] Compiling script...");
    let ast = match engine.compile(&script_content) {
        Ok(ast) => {
            tracing::debug!("[BACKTEST-RHAI] Script compiled successfully");
            ast
        }
        Err(e) => {
            tracing::error!("[BACKTEST-RHAI] Script compile error: {e}");
            tracing::error!("[BACKTEST-RHAI] Script content:\n{}", script_content);
            return Err(anyhow::anyhow!("Script compile error: {e}"));
        }
    };

    // Shared mutable state accessed by ctx methods (buy/sell/set/get)
    #[derive(Clone)]
    struct State {
        balance:     f64,
        position:    f64,   // +N = long N units, -N = short N units
        entry_price: f64,
        entry_index: i64,
        trades:      Vec<Trade>,
        kv:          std::collections::HashMap<String, f64>,
        pending_buy:  bool,
        pending_sell: bool,
        stop_loss:    f64,  // 0 = inactive
        take_profit:  f64,  // 0 = inactive
    }

    let state = Arc::new(Mutex::new(State {
        balance: initial_balance,
        position: 0.0,
        entry_price: 0.0,
        entry_index: 0,
        trades: Vec::new(),
        kv: std::collections::HashMap::new(),
        pending_buy: false,
        pending_sell: false,
        stop_loss: 0.0,
        take_profit: 0.0,
    }));

    // Clones for closures
    let closes_arc  = Arc::new(closes.clone());
    let volumes_arc = Arc::new(volumes.clone());
    let highs_arc   = Arc::new(highs.clone());
    let lows_arc    = Arc::new(lows.clone());

    // Pre-compute MACD for legacy scripts
    let (macd_line, signal_line, macd_hist) = compute_macd_series(&closes);

    let mut portfolio_values: Vec<f64> = vec![initial_balance];
    let mut peak = initial_balance;
    let mut max_dd = 0.0_f64;

    // Check if script has on_candle function
    let has_on_candle = ast.iter_functions().any(|f| f.name == "on_candle");
    let fn_names: Vec<_> = ast.iter_functions().map(|f| f.name.to_string()).collect();
    tracing::info!(
        "[BACKTEST-RHAI] Script API: has_on_candle={}, functions={:?}",
        has_on_candle, fn_names
    );

    for i in 0..candles.len() {
        let c = &candles[i];
        let ts = chrono::DateTime::from_timestamp_millis(c.open_time_ms)
            .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .unwrap_or_else(|| c.open_time_ms.to_string());

        // ── Stop-loss / Take-profit enforcement ──────────────────────────────
        {
            let mut s = state.lock().unwrap();
            if s.position > 0.0 {
                let price = c.close;
                let hit_sl = s.stop_loss  > 0.0 && price <= s.stop_loss;
                let hit_tp = s.take_profit > 0.0 && price >= s.take_profit;
                if hit_sl || hit_tp {
                    let fee_factor = 1.0 - fee_pct / 100.0;
                    let pos     = s.position;
                    let proceeds = pos * price * fee_factor;
                    let pnl      = proceeds - s.entry_price * pos;
                    s.trades.push(Trade {
                        timestamp: ts.clone(),
                        side: if hit_sl { "stop_loss".into() } else { "take_profit".into() },
                        price,
                        size: pos,
                        pnl,
                        debug: None,
                    });
                    s.balance     = proceeds;
                    s.position    = 0.0;
                    s.entry_price = 0.0;
                    s.stop_loss   = 0.0;
                    s.take_profit = 0.0;
                }
            } else if s.position < 0.0 {
                let price = c.close;
                let hit_sl = s.stop_loss  > 0.0 && price >= s.stop_loss;
                let hit_tp = s.take_profit > 0.0 && price <= s.take_profit;
                if hit_sl || hit_tp {
                    let fee_factor = 1.0 - fee_pct / 100.0;
                    let pos_abs = s.position.abs();
                    let pnl = (s.entry_price - price) * pos_abs * fee_factor;
                    s.trades.push(Trade {
                        timestamp: ts.clone(),
                        side: if hit_sl { "short_stop_loss".into() } else { "short_take_profit".into() },
                        price,
                        size: pos_abs,
                        pnl,
                        debug: None,
                    });
                    s.balance    += pnl;
                    s.position    = 0.0;
                    s.entry_price = 0.0;
                    s.stop_loss   = 0.0;
                    s.take_profit = 0.0;
                }
            }
        }

        if has_on_candle {
            // ── ctx-based API ────────────────────────────────────────
            let (cur_balance, cur_position, cur_entry_price, cur_entry_index) = {
                let s = state.lock().unwrap();
                (s.balance, s.position, s.entry_price, s.entry_index)
            };

            let cur_open_positions: i64 = if cur_position != 0.0 { 1 } else { 0 };

            // Build ctx map
            let mut ctx = Map::new();
            ctx.insert("close".into(),          Dynamic::from(c.close));
            ctx.insert("open".into(),           Dynamic::from(c.open));
            ctx.insert("high".into(),           Dynamic::from(c.high));
            ctx.insert("low".into(),            Dynamic::from(c.low));
            ctx.insert("volume".into(),         Dynamic::from(c.volume));
            ctx.insert("index".into(),          Dynamic::from(i as i64));
            ctx.insert("position".into(),       Dynamic::from(cur_position));
            ctx.insert("entry_price".into(),    Dynamic::from(cur_entry_price));
            ctx.insert("entry_index".into(),    Dynamic::from(cur_entry_index));
            ctx.insert("balance".into(),        Dynamic::from(cur_balance));
            ctx.insert("open_positions".into(), Dynamic::from(cur_open_positions));

            // close_at(idx) — returns close price at a past index
            let closes_c = closes_arc.clone();
            ctx.insert("close_at".into(), Dynamic::from(rhai::FnPtr::new("close_at")?));
            let _ = closes_c; // referenced via registered fn below

            // Register helper functions on the engine per-candle via scope vars
            // We pass large arrays via scope, not closures, for simplicity.
            let mut scope = Scope::new();
            scope.push("_closes",  closes_arc.to_vec());
            scope.push("_volumes", volumes_arc.to_vec());
            scope.push("_highs",   highs_arc.to_vec());
            scope.push("_lows",    lows_arc.to_vec());
            scope.push("_cur_idx", i as i64);
            scope.push("_state_buy",  false);
            scope.push("_state_sell", false);
            scope.push("_atr14",   atr14.to_vec());

            // Inject ctx as a scope variable
            scope.push("_ctx_map", Dynamic::from(ctx));

            // Build a wrapper script that calls on_candle with a ctx proxy
            // using scope-based accessors.
            let _wrapper = r#"
fn _rsi(period) {
    let n = _closes.len();
    if n < period + 1 { return 50.0; }
    let idx = _cur_idx;
    if idx < period { return 50.0; }
    let avg_gain = 0.0;
    let avg_loss = 0.0;
    let j = idx - period + 1;
    while j <= idx {
        if j == 0 { j += 1; continue; }
        let d = _closes[j] - _closes[j-1];
        if d > 0.0 { avg_gain += d; } else { avg_loss += -d; }
        j += 1;
    }
    avg_gain /= period;
    avg_loss /= period;
    if avg_loss == 0.0 { return 100.0; }
    100.0 - 100.0 / (1.0 + avg_gain / avg_loss)
}

fn _ema(period) {
    let idx = _cur_idx;
    if idx < period - 1 { return _closes[idx]; }
    let k = 2.0 / (period + 1);
    let start = (idx - period * 3).max(0);
    let mut e = _closes[start];
    let j = start + 1;
    while j <= idx {
        e = _closes[j] * k + e * (1.0 - k);
        j += 1;
    }
    e
}

fn _atr(period) {
    _atr14[_cur_idx]
}

fn _close_at(i) { _closes[i] }
fn _volume_at(i) { _volumes[i] }

// Proxy object with methods
let ctx = #{
    close:       _closes[_cur_idx],
    open:        _ctx_map["open"],
    high:        _ctx_map["high"],
    low:         _ctx_map["low"],
    volume:      _volumes[_cur_idx],
    index:       _cur_idx,
    position:    _ctx_map["position"],
    entry_price: _ctx_map["entry_price"],
    entry_index: _ctx_map["entry_index"],
    balance:     _ctx_map["balance"],
};

// Method-like closures via map function fields — not supported in Rhai maps.
// We use global fns instead and patch ctx after the call.
// The script calls ctx.rsi(n), ctx.ema(n) etc via Rhai method syntax.
"#;
            // This wrapper approach won't work for method calls on the map.
            // Instead register native functions so ctx.rsi(n) etc work.
            drop(scope);

            // Use a proper per-candle approach: register native fns with captured data
            let mut eng2 = Engine::new();
            eng2.set_max_operations(500_000);
            eng2.set_max_call_levels(64);

            // Capture data for closures
            let closes_fn  = closes_arc.clone();
            let _volumes_fn = volumes_arc.clone();
            let _atr14_fn  = atr14.to_vec();
            let state_buy  = state.clone();
            let state_sell = state.clone();
            let state_set  = state.clone();
            let state_get  = state.clone();
            let state_ssl  = state.clone();
            let state_stp  = state.clone();
            let cur_i      = i;

            // ctx.close_at(idx) → f64
            eng2.register_fn("close_at_impl", move |idx: i64| -> f64 {
                closes_fn.get(idx as usize).copied().unwrap_or(0.0)
            });

            let volumes_fn2 = volumes_arc.clone();
            eng2.register_fn("volume_at_impl", move |idx: i64| -> f64 {
                volumes_fn2.get(idx as usize).copied().unwrap_or(0.0)
            });

            // ctx.high_at(idx) → f64
            let highs_fn = highs_arc.clone();
            eng2.register_fn("high_at_impl", move |idx: i64| -> f64 {
                highs_fn.get(idx as usize).copied().unwrap_or(0.0)
            });

            // ctx.low_at(idx) → f64
            let lows_fn = lows_arc.clone();
            eng2.register_fn("low_at_impl", move |idx: i64| -> f64 {
                lows_fn.get(idx as usize).copied().unwrap_or(0.0)
            });

            // ctx.set_stop_loss(price)
            eng2.register_fn("set_stop_loss_impl", move |price: f64| {
                state_ssl.lock().unwrap().stop_loss = price;
            });

            // ctx.set_take_profit(price)
            eng2.register_fn("set_take_profit_impl", move |price: f64| {
                state_stp.lock().unwrap().take_profit = price;
            });

            // rsi(period) computed inline
            let closes_rsi = closes_arc.clone();
            eng2.register_fn("rsi_impl", move |period: i64| -> f64 {
                let period = period as usize;
                let idx = cur_i;
                if idx < period { return 50.0; }
                let mut avg_gain = 0.0_f64;
                let mut avg_loss = 0.0_f64;
                for j in (idx - period + 1)..=idx {
                    if j == 0 { continue; }
                    let d = closes_rsi[j] - closes_rsi[j - 1];
                    if d > 0.0 { avg_gain += d; } else { avg_loss += d.abs(); }
                }
                avg_gain /= period as f64;
                avg_loss /= period as f64;
                if avg_loss == 0.0 { 100.0 } else { 100.0 - 100.0 / (1.0 + avg_gain / avg_loss) }
            });

            // ema(period) computed inline
            let closes_ema = closes_arc.clone();
            eng2.register_fn("ema_impl", move |period: i64| -> f64 {
                let period = period as usize;
                let idx = cur_i;
                if period == 0 || idx == 0 { return closes_ema.get(idx).copied().unwrap_or(0.0); }
                let k = 2.0 / (period as f64 + 1.0);
                let start = idx.saturating_sub(period * 5);
                let mut e = closes_ema[start];
                for j in (start + 1)..=idx {
                    e = closes_ema[j] * k + e * (1.0 - k);
                }
                e
            });

            // atr(period) — Average True Range computed inline with correct period
            let highs_atr  = highs_arc.clone();
            let lows_atr   = lows_arc.clone();
            let closes_atr = closes_arc.clone();
            eng2.register_fn("atr_impl", move |period: i64| -> f64 {
                let period = (period.max(1)) as usize;
                let idx = cur_i;
                if idx == 0 { return 0.0; }
                let lookback = period * 3;
                let start = idx.saturating_sub(lookback);
                let mut tr_vals: Vec<f64> = Vec::new();
                for i in (start + 1)..=idx {
                    let tr = (highs_atr[i] - lows_atr[i])
                        .max((highs_atr[i] - closes_atr[i - 1]).abs())
                        .max((lows_atr[i]  - closes_atr[i - 1]).abs());
                    tr_vals.push(tr);
                }
                if tr_vals.is_empty() { return 0.0; }
                if tr_vals.len() < period {
                    return tr_vals.iter().sum::<f64>() / tr_vals.len() as f64;
                }
                let mut atr = tr_vals[..period].iter().sum::<f64>() / period as f64;
                for j in period..tr_vals.len() {
                    atr = (atr * (period - 1) as f64 + tr_vals[j]) / period as f64;
                }
                atr
            });

            // sma(period) - Simple Moving Average
            let closes_sma = closes_arc.clone();
            eng2.register_fn("sma_impl", move |period: i64| -> f64 {
                let period = period as usize;
                if period == 0 { return 0.0; }
                let idx = cur_i;
                if idx + 1 < period {
                    let slice = &closes_sma[..=idx];
                    return slice.iter().sum::<f64>() / slice.len() as f64;
                }
                let start = idx + 1 - period;
                closes_sma[start..=idx].iter().sum::<f64>() / period as f64
            });

            // macd_hist(fast, slow, signal) - MACD histogram
            let closes_macd = closes_arc.clone();
            eng2.register_fn("macd_hist_impl", move |fast: i64, slow: i64, signal: i64| -> f64 {
                let idx = cur_i;
                macd_hist_value(&closes_macd, idx, fast as usize, slow as usize, signal as usize)
            });

            // bb_middle(period) - Bollinger Band middle line (SMA)
            let closes_bbm = closes_arc.clone();
            eng2.register_fn("bb_middle_impl", move |period: i64| -> f64 {
                let period = period as usize;
                if period == 0 { return 0.0; }
                let idx = cur_i;
                if idx + 1 < period {
                    let slice = &closes_bbm[..=idx];
                    return slice.iter().sum::<f64>() / slice.len() as f64;
                }
                let start = idx + 1 - period;
                closes_bbm[start..=idx].iter().sum::<f64>() / period as f64
            });

            // bb_upper(period, mult) - Bollinger upper band = SMA + mult * StdDev
            let closes_bbu = closes_arc.clone();
            eng2.register_fn("bb_upper_impl", move |period: i64, mult: f64| -> f64 {
                let period = period as usize;
                if period == 0 { return 0.0; }
                let idx = cur_i;
                let start = if idx + 1 >= period { idx + 1 - period } else { 0 };
                let slice = &closes_bbu[start..=idx];
                let mean = slice.iter().sum::<f64>() / slice.len() as f64;
                let var  = slice.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / slice.len() as f64;
                mean + mult * var.sqrt()
            });

            // bb_lower(period, mult) - Bollinger lower band = SMA - mult * StdDev
            let closes_bbl = closes_arc.clone();
            eng2.register_fn("bb_lower_impl", move |period: i64, mult: f64| -> f64 {
                let period = period as usize;
                if period == 0 { return 0.0; }
                let idx = cur_i;
                let start = if idx + 1 >= period { idx + 1 - period } else { 0 };
                let slice = &closes_bbl[start..=idx];
                let mean = slice.iter().sum::<f64>() / slice.len() as f64;
                let var  = slice.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / slice.len() as f64;
                mean - mult * var.sqrt()
            });

            // bb_width(period, mult) - BB width as % of middle (volatility measure)
            let closes_bbw = closes_arc.clone();
            eng2.register_fn("bb_width_impl", move |period: i64, mult: f64| -> f64 {
                let period = period as usize;
                if period == 0 { return 0.0; }
                let idx = cur_i;
                let start = if idx + 1 >= period { idx + 1 - period } else { 0 };
                let slice = &closes_bbw[start..=idx];
                let mean = slice.iter().sum::<f64>() / slice.len() as f64;
                let var  = slice.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / slice.len() as f64;
                let std  = var.sqrt();
                if mean > 0.0 { (2.0 * mult * std) / mean * 100.0 } else { 0.0 }
            });

            // stoch_k(period) - Stochastic %K = (close - lowest_low) / (highest_high - lowest_low) * 100
            let highs_stoch = highs_arc.clone();
            let lows_stoch  = lows_arc.clone();
            let closes_stoch = closes_arc.clone();
            eng2.register_fn("stoch_k_impl", move |period: i64| -> f64 {
                let period = period as usize;
                if period == 0 { return 50.0; }
                let idx = cur_i;
                let start = if idx + 1 >= period { idx + 1 - period } else { 0 };
                let highest = highs_stoch[start..=idx].iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                let lowest  = lows_stoch[start..=idx].iter().cloned().fold(f64::INFINITY, f64::min);
                let close   = closes_stoch.get(idx).copied().unwrap_or(0.0);
                if (highest - lowest).abs() < 1e-10 { return 50.0; }
                (close - lowest) / (highest - lowest) * 100.0
            });

            // vwap() - Volume-Weighted Average Price (from candle start of series or last 100 bars)
            let closes_vwap  = closes_arc.clone();
            let volumes_vwap = volumes_arc.clone();
            eng2.register_fn("vwap_impl", move || -> f64 {
                let idx = cur_i;
                let start = idx.saturating_sub(100);
                let mut sum_pv = 0.0_f64;
                let mut sum_v  = 0.0_f64;
                for j in start..=idx {
                    let v = volumes_vwap.get(j).copied().unwrap_or(0.0);
                    let p = closes_vwap.get(j).copied().unwrap_or(0.0);
                    sum_pv += p * v;
                    sum_v  += v;
                }
                if sum_v > 0.0 { sum_pv / sum_v } else { closes_vwap.get(idx).copied().unwrap_or(0.0) }
            });

            // stddev(period) - Standard deviation of closes
            let closes_std = closes_arc.clone();
            eng2.register_fn("stddev_impl", move |period: i64| -> f64 {
                let period = period as usize;
                if period == 0 { return 0.0; }
                let idx = cur_i;
                let start = if idx + 1 >= period { idx + 1 - period } else { 0 };
                let slice = &closes_std[start..=idx];
                let mean = slice.iter().sum::<f64>() / slice.len() as f64;
                let var  = slice.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / slice.len() as f64;
                var.sqrt()
            });

            // ctx.log(msg) - log a message from the strategy script
            eng2.register_fn("log_impl", move |msg: rhai::Dynamic| {
                tracing::info!("[STRATEGY candle={}] {}", cur_i, msg);
            });

            // ctx.buy(size) — size 0.0-1.0 = fraction of balance; supports averaging in
            let sb = state_buy.clone();
            let buy_price = c.close;
            let _buy_ts   = ts.clone();
            let buy_fee   = fee_pct;
            let max_ep    = max_entry_price;
            let sz_mode   = sizing_mode.to_string();
            let sz_val    = sizing_value;
            eng2.register_fn("buy_impl", move |size: f64| {
                let mut s = sb.lock().unwrap();
                // Skip if price exceeds max entry price threshold
                if let Some(max_entry) = max_ep {
                    if buy_price > max_entry {
                        return;
                    }
                }
                // If short, close the short first
                if s.position < 0.0 {
                    let fee_factor = 1.0 - buy_fee / 100.0;
                    let pos_abs = s.position.abs();
                    let pnl = (s.entry_price - buy_price) * pos_abs * fee_factor;
                    s.trades.push(Trade {
                        timestamp: String::new(),
                        side: "buy_cover".into(),
                        price: buy_price,
                        size: pos_abs,
                        pnl,
                        debug: None,
                    });
                    s.balance    += pnl;
                    s.position    = 0.0;
                    s.entry_price = 0.0;
                    s.stop_loss   = 0.0;
                    s.take_profit = 0.0;
                }
                if s.balance > 0.0 {
                    let amount = if sz_mode == "fixed" {
                        sz_val.min(s.balance)
                    } else {
                        let max_frac = sz_val.max(0.0).min(1.0);
                        let frac = size.max(0.0).min(1.0).min(max_frac);
                        s.balance * frac
                    };
                    if amount <= 0.0 { return; }
                    let fee_factor = 1.0 - buy_fee / 100.0;
                    let qty = (amount * fee_factor) / buy_price;
                    if s.position == 0.0 {
                        // Fresh entry
                        s.entry_price = buy_price;
                        s.entry_index = cur_i as i64;
                        s.stop_loss   = 0.0;
                        s.take_profit = 0.0;
                    } else {
                        // Adding to existing long: weighted average entry price
                        let total_cost = s.entry_price * s.position + buy_price * qty;
                        s.entry_price = total_cost / (s.position + qty);
                    }
                    s.position += qty;
                    s.balance  -= amount;
                }
            });

            // ctx.sell(size) — size 1.0 = close full position; if flat, opens a short
            let ss = state_sell.clone();
            let sell_price = c.close;
            let sell_ts    = ts.clone();
            let sell_fee   = fee_pct;
            eng2.register_fn("sell_impl", move |_size: f64| {
                let mut s = ss.lock().unwrap();
                if s.position > 0.0 {
                    // Close long
                    let fee_factor = 1.0 - sell_fee / 100.0;
                    let pos = s.position;
                    let gross = pos * sell_price;
                    let proceeds = gross * fee_factor;
                    let pnl = proceeds - s.entry_price * pos;
                    s.trades.push(Trade {
                        timestamp: sell_ts.clone(),
                        side: "sell".into(),
                        price: sell_price,
                        size: pos,
                        pnl,
                        debug: None,
                    });
                    s.balance     = proceeds;
                    s.position    = 0.0;
                    s.entry_price = 0.0;
                    s.stop_loss   = 0.0;
                    s.take_profit = 0.0;
                } else if s.position < 0.0 {
                    // Close short (cover)
                    let fee_factor = 1.0 - sell_fee / 100.0;
                    let pos_abs = s.position.abs();
                    let pnl = (s.entry_price - sell_price) * pos_abs * fee_factor;
                    s.trades.push(Trade {
                        timestamp: sell_ts.clone(),
                        side: "buy_cover".into(),
                        price: sell_price,
                        size: pos_abs,
                        pnl,
                        debug: None,
                    });
                    s.balance    += pnl;
                    s.position    = 0.0;
                    s.entry_price = 0.0;
                    s.stop_loss   = 0.0;
                    s.take_profit = 0.0;
                } else if s.balance > 0.0 {
                    // Flat → open short (sell borrowed units)
                    let fee_factor = 1.0 - sell_fee / 100.0;
                    let qty = (s.balance * fee_factor) / sell_price;
                    s.position    = -qty;
                    s.entry_price = sell_price;
                    s.entry_index = cur_i as i64;
                    s.stop_loss   = 0.0;
                    s.take_profit = 0.0;
                }
            });

            // ctx.short(size) — explicit short-open alias (same as sell when flat)
            let state_short = state.clone();
            let short_price = c.close;
            let short_ts    = ts.clone();
            let short_fee   = fee_pct;
            eng2.register_fn("short_impl", move |_size: f64| {
                let mut s = state_short.lock().unwrap();
                if s.position > 0.0 {
                    // Close long first
                    let fee_factor = 1.0 - short_fee / 100.0;
                    let pos = s.position;
                    let proceeds = pos * short_price * fee_factor;
                    let pnl = proceeds - s.entry_price * pos;
                    s.trades.push(Trade {
                        timestamp: short_ts.clone(),
                        side: "sell".into(),
                        price: short_price,
                        size: pos,
                        pnl,
                        debug: None,
                    });
                    s.balance     = proceeds;
                    s.position    = 0.0;
                    s.entry_price = 0.0;
                }
                if s.position == 0.0 && s.balance > 0.0 {
                    let fee_factor = 1.0 - short_fee / 100.0;
                    let qty = (s.balance * fee_factor) / short_price;
                    s.position    = -qty;
                    s.entry_price = short_price;
                    s.entry_index = cur_i as i64;
                    s.stop_loss   = 0.0;
                    s.take_profit = 0.0;
                }
            });

            // ctx.set(key, val) / ctx.get(key, default)
            let sset = state_set.clone();
            eng2.register_fn("set_impl", move |key: String, val: f64| {
                sset.lock().unwrap().kv.insert(key, val);
            });
            let sget = state_get.clone();
            eng2.register_fn("get_impl", move |key: String, default: f64| -> f64 {
                sget.lock().unwrap().kv.get(&key).copied().unwrap_or(default)
            });

            // Re-compile the script with our engine + a shim that maps ctx.* methods
            // We prepend a shim that wraps map methods → global fns, then calls on_candle.
            let (cur_bal, cur_pos, cur_ep, cur_ei) = {
                let s = state.lock().unwrap();
                (s.balance, s.position, s.entry_price, s.entry_index)
            };

            let _shim = format!(r#"
// Method proxies — Rhai calls ctx.rsi(n) as a method on the map,
// which routes to the registered native fn via the index trick.
// We define standalone wrappers then inject them.
fn on_candle_shim() {{
    let ctx = #{{}};
    ctx.close       = {close};
    ctx.open        = {open};
    ctx.high        = {high};
    ctx.low         = {low};
    ctx.volume      = {volume};
    ctx.index       = {index};
    ctx.position    = {position};
    ctx.entry_price = {entry_price};
    ctx.entry_index = {entry_index};
    ctx.balance     = {balance};
    // Inject method-like closures as function pointers isn't easy in Rhai.
    // Instead we expose global functions and override with shim methods:
    on_candle(ctx);
}}
on_candle_shim();
"#,
                close       = c.close,
                open        = c.open,
                high        = c.high,
                low         = c.low,
                volume      = c.volume,
                index       = i,
                position    = cur_pos,
                entry_price = cur_ep,
                entry_index = cur_ei,
                balance     = cur_bal,
            );

            // Override ctx method calls: ctx.rsi(n) → rsi_impl(n), etc.
            // In Rhai, method calls on a map go to the map's function field if it exists,
            // otherwise fall through to global functions with the same name.
            // We register the functions as global so ctx.rsi(14) → rsi_impl(14).
            // But Rhai method calls pass the object as first arg — we need to
            // handle this differently.
            //
            // The cleanest approach for Rhai is to make ctx a custom struct
            // exposed via the plugin API. We use a simpler shim: rewrite the
            // user script to replace ctx.rsi(n) → rsi_impl(n), ctx.ema(n) → ema_impl(n), etc.

            let patched_script = script_content
                .replace("ctx.rsi(",            "rsi_impl(")
                .replace("ctx.ema(",            "ema_impl(")
                .replace("ctx.atr(",            "atr_impl(")
                .replace("ctx.sma(",            "sma_impl(")
                .replace("ctx.macd_hist(",      "macd_hist_impl(")
                .replace("ctx.close_at(",       "close_at_impl(")
                .replace("ctx.high_at(",        "high_at_impl(")
                .replace("ctx.low_at(",         "low_at_impl(")
                .replace("ctx.volume_at(",      "volume_at_impl(")
                .replace("ctx.set_stop_loss(",  "set_stop_loss_impl(")
                .replace("ctx.set_take_profit(","set_take_profit_impl(")
                .replace("ctx.bb_upper(",       "bb_upper_impl(")
                .replace("ctx.bb_lower(",       "bb_lower_impl(")
                .replace("ctx.bb_middle(",      "bb_middle_impl(")
                .replace("ctx.bb_width(",       "bb_width_impl(")
                .replace("ctx.stoch_k(",        "stoch_k_impl(")
                .replace("ctx.vwap()",          "vwap_impl()")
                .replace("ctx.stddev(",         "stddev_impl(")
                .replace("ctx.short(",          "short_impl(")
                .replace("ctx.buy(",            "buy_impl(")
                .replace("ctx.sell(",           "sell_impl(")
                .replace("ctx.set(",            "set_impl(")
                .replace("ctx.get(",            "get_impl(")
                .replace("ctx.log(",            "log_impl(");

            let full_script = format!(r#"
{patched_script}

let ctx = #{{}};
ctx.close          = {close};
ctx.open           = {open};
ctx.high           = {high};
ctx.low            = {low};
ctx.volume         = {volume};
ctx.index          = {index};
ctx.position       = {position};
ctx.entry_price    = {entry_price};
ctx.entry_index    = {entry_index};
ctx.balance        = {balance};
ctx.open_positions = {open_positions};
on_candle(ctx);
"#,
                patched_script  = patched_script,
                close           = c.close,
                open            = c.open,
                high            = c.high,
                low             = c.low,
                volume          = c.volume,
                index           = i,
                position        = cur_pos,
                entry_price     = cur_ep,
                entry_index     = cur_ei,
                balance         = cur_bal,
                open_positions  = cur_open_positions,
            );

            let ast2 = match eng2.compile(&full_script) {
                Ok(a) => a,
                Err(e) => {
                    if i == 0 {
                        tracing::error!("[BACKTEST-RHAI] Failed to compile patched script at candle {}: {}", i, e);
                        tracing::debug!("[BACKTEST-RHAI] Patched script:\n{}", full_script);
                    }
                    continue;
                }
            };
            let mut scope2 = Scope::new();
            if let Err(e) = eng2.eval_ast_with_scope::<Dynamic>(&mut scope2, &ast2) {
                if i == 0 {
                    tracing::warn!("[BACKTEST-RHAI] on_candle execution error at candle {}: {}", i, e);
                }
            }
        } else {
            // ── Legacy signal-based API ───────────────────────────────
            if i == 0 {
                tracing::debug!("[BACKTEST-RHAI] Using legacy signal-based API");
            }
            let rsi14 = compute_rsi_series(&closes, 14);
            let (cur_balance, cur_position) = {
                let s = state.lock().unwrap();
                (s.balance, s.position)
            };

            let mut scope = Scope::new();
            scope.push("open",      c.open);
            scope.push("high",      c.high);
            scope.push("low",       c.low);
            scope.push("close",     c.close);
            scope.push("volume",    c.volume);
            scope.push("ts",        ts.clone());
            scope.push("rsi",       rsi14[i]);
            scope.push("macd",      macd_line[i]);
            scope.push("signal",    signal_line[i]);
            scope.push("macd_hist", macd_hist[i]);
            scope.push("balance",   cur_balance);
            scope.push("position",  cur_position);

            let signal_val: String = match engine.eval_ast_with_scope::<Dynamic>(&mut scope, &ast) {
                Ok(v) => {
                    let sig = v.try_cast::<String>().unwrap_or_else(|| "hold".into());
                    if i == 0 {
                        tracing::debug!("[BACKTEST-RHAI] Candle 0: eval returned signal={:?}", sig);
                    }
                    sig
                }
                Err(e) => {
                    if i == 0 {
                        tracing::warn!("[BACKTEST-RHAI] Candle 0 eval error: {}, checking scope for signal var", e);
                    }
                    scope.get_value::<String>("signal").unwrap_or_else(|| "hold".into())
                }
            };

            let fee_factor = 1.0 - fee_pct / 100.0;
            let mut s = state.lock().unwrap();
            match signal_val.to_lowercase().as_str() {
                "buy" if s.position == 0.0 && s.balance > 0.0 => {
                    let qty = (s.balance * fee_factor) / c.close;
                    s.position    = qty;
                    s.balance     = 0.0;
                    s.entry_price = c.close;
                }
                "sell" if s.position > 0.0 => {
                    let pos      = s.position;
                    let gross    = pos * c.close;
                    let proceeds = gross * fee_factor;
                    let pnl      = proceeds - s.entry_price * pos;
                    s.trades.push(Trade {
                        timestamp: ts.clone(),
                        side:  "sell".into(),
                        price: c.close,
                        size:  pos,
                        pnl,
                        debug: None,
                    });
                    s.balance     = proceeds;
                    s.position    = 0.0;
                    s.entry_price = 0.0;
                }
                _ => {}
            }
        }

        // Portfolio equity
        let (cur_balance, cur_position) = {
            let s = state.lock().unwrap();
            (s.balance, s.position)
        };
        let equity = cur_balance + cur_position * c.close;
        portfolio_values.push(equity);
        if equity > peak { peak = equity; }
        let dd = (peak - equity) / peak * 100.0;
        if dd > max_dd { max_dd = dd; }
    }

    // Extract final state
    let mut s = state.lock().unwrap();

    // Close any open position at last price
    if s.position > 0.0 && !candles.is_empty() {
        let last = candles.last().unwrap();
        let fee_factor = 1.0 - fee_pct / 100.0;
        let pos = s.position;
        let proceeds = pos * last.close * fee_factor;
        let pnl = proceeds - s.entry_price * pos;
        s.trades.push(Trade {
            timestamp: chrono::DateTime::from_timestamp_millis(last.open_time_ms)
                .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
                .unwrap_or_default(),
            side: "close".into(),
            price: last.close,
            size: pos,
            pnl,
            debug: None,
        });
        s.balance = proceeds;
    }

    let final_value = s.balance;
    let trades      = std::mem::take(&mut s.trades);
    drop(s);
    let total_return_pct = (final_value / initial_balance - 1.0) * 100.0;
    let total_trades = trades.len() as u32;

    let wins = trades.iter().filter(|t| t.pnl > 0.0).count();
    let win_rate_pct = if total_trades > 0 {
        wins as f64 / total_trades as f64 * 100.0
    } else {
        0.0
    };

    // Sharpe ratio (annualized daily returns)
    let sharpe_ratio = if portfolio_values.len() > 1 {
        let daily_returns: Vec<f64> = portfolio_values
            .windows(2)
            .map(|w| (w[1] / w[0] - 1.0))
            .collect();
        let mean = daily_returns.iter().sum::<f64>() / daily_returns.len() as f64;
        let variance = daily_returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>()
            / daily_returns.len() as f64;
        let std_dev = variance.sqrt();
        if std_dev > 0.0 { (mean / std_dev) * (252.0_f64).sqrt() } else { 0.0 }
    } else {
        0.0
    };

    // 5 worst trades
    let mut sorted_trades: Vec<&Trade> = trades.iter().collect();
    sorted_trades.sort_by(|a, b| a.pnl.partial_cmp(&b.pnl).unwrap_or(std::cmp::Ordering::Equal));
    let worst_trades: Vec<WorstTrade> = sorted_trades
        .iter()
        .take(5)
        .map(|t| WorstTrade {
            timestamp: t.timestamp.clone(),
            side: t.side.clone(),
            price: t.price,
            pnl: t.pnl,
        })
        .collect();

    // All trades with cumulative balance for equity curve
    let mut running_balance = initial_balance;
    let all_trades: Vec<AllTrade> = trades.iter().map(|t| {
        running_balance += t.pnl;
        AllTrade {
            timestamp: t.timestamp.clone(),
            side: t.side.clone(),
            price: t.price,
            size: t.size,
            pnl: t.pnl,
            balance: running_balance,
            debug: None,
        }
    }).collect();

    let analysis = build_analysis(total_return_pct, sharpe_ratio, max_dd, win_rate_pct, total_trades);

    Ok(BacktestMetrics {
        total_return_pct,
        sharpe_ratio,
        max_drawdown_pct: max_dd,
        win_rate_pct,
        total_trades,
        worst_trades,
        all_trades,
        analysis,
        avg_token_price: None,
        correct_direction_pct: None,
        break_even_win_rate: None,
        markets_tested: None, flat_debugs: vec![],
    })
}

fn build_analysis(
    total_return_pct: f64,
    sharpe_ratio: f64,
    max_drawdown_pct: f64,
    win_rate_pct: f64,
    total_trades: u32,
) -> String {
    let return_comment = if total_return_pct > 0.0 {
        format!("Positive return of {total_return_pct:.2}% achieved.")
    } else {
        format!("Negative return of {total_return_pct:.2}% — strategy needs tuning.")
    };
    let sharpe_comment = if sharpe_ratio > 1.5 {
        "excellent risk-adjusted performance"
    } else if sharpe_ratio > 1.0 {
        "acceptable risk-adjusted performance"
    } else {
        "below the 1.0 threshold — consider reducing volatility"
    };
    let drawdown_comment = if max_drawdown_pct < 10.0 {
        "within safe limits"
    } else if max_drawdown_pct < 20.0 {
        "moderate — consider tighter stop-losses"
    } else {
        "high — add stop-losses to protect capital"
    };
    let trade_comment = if total_trades == 0 {
        "The strategy generated no trades — check buy/sell signal conditions in the script."
            .to_string()
    } else {
        format!("{total_trades} trades executed. Win rate {win_rate_pct:.1}%.")
    };

    format!(
        "{return_comment} Sharpe ratio of {sharpe_ratio:.2} ({sharpe_comment}). \
        Max drawdown {max_drawdown_pct:.2}% ({drawdown_comment}). {trade_comment}"
    )
}

// ── Polymarket Binary Engine ─────────────────────────────────────────────────

/// Convert interval string to minutes (used for binary window size).
fn parse_interval_to_minutes(interval: &str) -> usize {
    match interval {
        "1m"  => 1,   "2m"  => 2,   "3m"  => 3,   "4m"  => 4,   "5m"  => 5,
        "10m" => 10,  "15m" => 15,  "30m" => 30,
        "1h"  => 60,  "2h"  => 120, "4h"  => 240,  "6h"  => 360, "12h" => 720,
        "1d"  => 1440,
        other => {
            if let Some(n) = other.strip_suffix('m').and_then(|s| s.parse::<usize>().ok()) {
                n
            } else if let Some(n) = other.strip_suffix('h').and_then(|s| s.parse::<usize>().ok()) {
                n * 60
            } else {
                5
            }
        }
    }
}

/// Piecewise-linear token pricing model from the observed live Polymarket behavior.
/// Input: absolute 4-candle momentum percentage (|delta|).
/// Output: estimated YES token entry price in dollars (0.50..0.97).
/// When you know which direction the market is moving, the token is no longer $0.50 —
/// it costs more to buy the "obvious" outcome, compressing the payout.
fn polymarket_token_price(momentum_abs_pct: f64) -> f64 {
    let d = momentum_abs_pct;
    if d < 0.005 {
        0.50
    } else if d < 0.02 {
        0.50 + (d - 0.005) / (0.02 - 0.005) * 0.05
    } else if d < 0.05 {
        0.55 + (d - 0.02) / (0.05 - 0.02) * 0.10
    } else if d < 0.10 {
        0.65 + (d - 0.05) / (0.10 - 0.05) * 0.15
    } else if d < 0.15 {
        0.80 + (d - 0.10) / (0.15 - 0.10) * 0.12
    } else {
        (0.92_f64 + ((d - 0.15) / 0.10 * 0.05).min(0.05)).min(0.97)
    }
}

fn build_binary_analysis(
    total_return_pct: f64,
    win_rate_pct: f64,
    total_trades: u32,
    avg_token_price: Option<f64>,
    correct_direction_pct: Option<f64>,
    break_even_win_rate: Option<f64>,
    window_minutes: usize,
) -> String {
    let edge = match (avg_token_price, break_even_win_rate) {
        (Some(avg), Some(bev)) => {
            let edge_pct = win_rate_pct - bev;
            if edge_pct > 5.0 {
                format!("Positive edge of +{edge_pct:.1}% above break-even ({bev:.1}%). \
                    Avg token price ${avg:.3} — market is pricing the signal fairly.")
            } else if edge_pct > 0.0 {
                format!("Slight edge of +{edge_pct:.1}% above break-even ({bev:.1}%). \
                    Avg token price ${avg:.3} — strategy is marginally profitable.")
            } else {
                format!("Negative edge of {edge_pct:.1}% vs break-even ({bev:.1}%). \
                    Avg token price ${avg:.3} — strategy loses on market friction.")
            }
        }
        _ => String::new(),
    };

    let direction_comment = correct_direction_pct
        .map(|pct| format!("Correct direction: {pct:.1}%."))
        .unwrap_or_default();

    let trade_comment = if total_trades == 0 {
        "No bets placed — check signal conditions.".to_string()
    } else {
        format!("{total_trades} {window_minutes}-min binary bets. Win rate {win_rate_pct:.1}%.")
    };

    let return_comment = if total_return_pct >= 0.0 {
        format!("Return +{total_return_pct:.2}%.")
    } else {
        format!("Return {total_return_pct:.2}%.")
    };

    format!("{return_comment} {trade_comment} {direction_comment} {edge}")
}

/// Run Polymarket binary backtesting engine.
///
/// Uses 1-minute Binance candles as the underlying data source.
/// For each candle the script can call ctx.buy(frac) (bet YES: price goes UP)
/// or ctx.sell(frac) (bet NO: price goes DOWN).
/// After `window_candles` (= window_minutes, since 1m data) the position
/// auto-resolves: if BTC moved in the predicted direction the bet pays
/// (stake / token_price) * $1.00 minus fee; otherwise the stake is lost.
///
/// Extra ctx fields available to the script:
///   ctx.token_price    — estimated YES token entry price (piecewise model)
///   ctx.window_minutes — resolution window in minutes
fn run_polymarket_binary_backtest(
    script_content: String,
    candles: Vec<Candle>,
    initial_balance: f64,
    fee_pct: f64,
    window_candles: usize,
) -> anyhow::Result<BacktestMetrics> {
    use rhai::Engine;
    use std::sync::{Arc, Mutex};

    if candles.is_empty() {
        return Err(anyhow::anyhow!("No candle data available for binary backtest"));
    }

    // ── Aggregate 1m candles into window-sized candles ──────────────────────
    // The strategy was written for N-minute candles (e.g. 5m) but we fetch 1m data.
    // We OHLCV-aggregate every `window_candles` 1m candles into a single candle so
    // that momentum thresholds and indicator periods behave as intended.
    // Resolution is still checked at each window boundary (close vs open price).
    let aggregated: Vec<Candle> = if window_candles <= 1 {
        candles.clone()
    } else {
        candles
            .chunks(window_candles)
            .filter(|chunk| !chunk.is_empty())
            .map(|chunk| {
                let open          = chunk[0].open;
                let close         = chunk[chunk.len() - 1].close;
                let high          = chunk.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
                let low           = chunk.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
                let volume        = chunk.iter().map(|c| c.volume).sum();
                let open_time_ms  = chunk[0].open_time_ms;
                Candle { open, high, low, close, volume, open_time_ms }
            })
            .collect()
    };

    let closes:  Vec<f64> = aggregated.iter().map(|c| c.close).collect();
    let highs:   Vec<f64> = aggregated.iter().map(|c| c.high).collect();
    let lows:    Vec<f64> = aggregated.iter().map(|c| c.low).collect();
    let volumes: Vec<f64> = aggregated.iter().map(|c| c.volume).collect();
    let candles = aggregated; // strategy now sees aggregated candles

    // Check the script has on_candle
    let check_engine = Engine::new();
    let has_on_candle = match check_engine.compile(&script_content) {
        Ok(ast) => ast.iter_functions().any(|f| f.name == "on_candle"),
        Err(e)  => return Err(anyhow::anyhow!("Script compile error: {e}")),
    };
    if !has_on_candle {
        return Err(anyhow::anyhow!(
            "Binary backtest requires an on_candle(ctx) function. \
            Legacy signal-based scripts are not supported in binary mode."
        ));
    }

    #[derive(Clone)]
    struct BinaryState {
        balance:      f64,
        // open bet fields
        bet_active:      bool,
        bet_direction:   i8,    // +1 = YES/up, -1 = NO/down
        bet_entry_close: f64,
        bet_entry_idx:   usize,
        bet_token_price: f64,
        bet_stake:       f64,
        // stats
        trades:          Vec<Trade>,
        kv:              std::collections::HashMap<String, f64>,
        total_correct:   u32,
        total_resolved:  u32,
        sum_token_price: f64,
    }

    let state = Arc::new(Mutex::new(BinaryState {
        balance: initial_balance,
        bet_active: false, bet_direction: 0,
        bet_entry_close: 0.0, bet_entry_idx: 0,
        bet_token_price: 0.0, bet_stake: 0.0,
        trades: Vec::new(),
        kv: std::collections::HashMap::new(),
        total_correct: 0, total_resolved: 0, sum_token_price: 0.0,
    }));

    let closes_arc  = Arc::new(closes.clone());
    let volumes_arc = Arc::new(volumes.clone());
    let highs_arc   = Arc::new(highs.clone());
    let lows_arc    = Arc::new(lows.clone());

    let mut portfolio_values: Vec<f64> = vec![initial_balance];
    let mut peak   = initial_balance;
    let mut max_dd = 0.0_f64;

    // Patch script once (replace ctx.* method calls with global fn names)
    let patched_script = script_content
        .replace("ctx.rsi(",             "rsi_impl(")
        .replace("ctx.ema(",             "ema_impl(")
        .replace("ctx.atr(",             "atr_impl(")
        .replace("ctx.sma(",             "sma_impl(")
        .replace("ctx.macd_hist(",       "macd_hist_impl(")
        .replace("ctx.close_at(",        "close_at_impl(")
        .replace("ctx.high_at(",         "high_at_impl(")
        .replace("ctx.low_at(",          "low_at_impl(")
        .replace("ctx.volume_at(",       "volume_at_impl(")
        .replace("ctx.set_stop_loss(",   "set_stop_loss_impl(")
        .replace("ctx.set_take_profit(", "set_take_profit_impl(")
        .replace("ctx.buy(",             "buy_impl(")
        .replace("ctx.sell(",            "sell_impl(")
        .replace("ctx.short(",           "sell_impl(")
        .replace("ctx.set(",             "set_impl(")
        .replace("ctx.get(",             "get_impl(")
        .replace("ctx.log(",             "log_impl(");

    for i in 0..candles.len() {
        let c  = &candles[i];
        let ts = chrono::DateTime::from_timestamp_millis(c.open_time_ms)
            .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .unwrap_or_else(|| c.open_time_ms.to_string());

        // ── Auto-resolve expired bet ──────────────────────────────────────────
        {
            let mut s = state.lock().unwrap();
            if s.bet_active && (i.saturating_sub(s.bet_entry_idx)) >= window_candles {
                let went_up      = c.close > s.bet_entry_close;
                let direction    = s.bet_direction;
                let token_price  = s.bet_token_price;
                let stake        = s.bet_stake;
                let won = (direction > 0 && went_up) || (direction < 0 && !went_up);

                let tokens       = stake / token_price;
                let gross_payout = if won { tokens } else { 0.0 };
                let net_payout   = gross_payout * (1.0 - fee_pct / 100.0);
                let pnl          = net_payout - stake;

                s.balance += net_payout;
                if won { s.total_correct += 1; }
                s.total_resolved += 1;

                let side_str = match (direction > 0, won) {
                    (true,  true)  => "yes_win",
                    (true,  false) => "yes_loss",
                    (false, true)  => "no_win",
                    (false, false) => "no_loss",
                };
                s.trades.push(Trade {
                    timestamp: ts.clone(),
                    side:  side_str.into(),
                    price: token_price,
                    size:  tokens,
                    pnl,
                    debug: None,
                });

                s.bet_active    = false;
                s.bet_direction = 0;
            }
        }

        // Compute 4-candle momentum for token price model
        let mom4_abs = if i >= 4 {
            let c4 = closes[i - 4];
            if c4 > 0.0 { ((c.close - c4) / c4 * 100.0).abs() } else { 0.0 }
        } else { 0.0 };
        let yes_token_price = polymarket_token_price(mom4_abs);

        // Read state for ctx injection
        let (cur_balance, bet_active, bet_dir, bet_ep, bet_ei) = {
            let s = state.lock().unwrap();
            (s.balance, s.bet_active, s.bet_direction, s.bet_entry_close, s.bet_entry_idx)
        };
        let cur_position    = if bet_active { bet_dir as f64 } else { 0.0 };
        let cur_entry_price = if bet_active { bet_ep } else { 0.0 };
        let cur_entry_index = if bet_active { bet_ei as i64 } else { 0i64 };

        // Build a fresh engine per candle with captured state
        let mut eng = Engine::new();
        eng.set_max_operations(500_000);
        eng.set_max_call_levels(64);

        // ── Indicators ───────────────────────────────────────────────────────
        let cl = closes_arc.clone();
        eng.register_fn("close_at_impl",  move |idx: i64| -> f64 { cl.get(idx as usize).copied().unwrap_or(0.0) });
        let vl = volumes_arc.clone();
        eng.register_fn("volume_at_impl", move |idx: i64| -> f64 { vl.get(idx as usize).copied().unwrap_or(0.0) });
        let hl = highs_arc.clone();
        eng.register_fn("high_at_impl",   move |idx: i64| -> f64 { hl.get(idx as usize).copied().unwrap_or(0.0) });
        let ll = lows_arc.clone();
        eng.register_fn("low_at_impl",    move |idx: i64| -> f64 { ll.get(idx as usize).copied().unwrap_or(0.0) });

        let cr = closes_arc.clone();
        eng.register_fn("rsi_impl", move |period: i64| -> f64 {
            let period = period as usize;
            if i < period { return 50.0; }
            let mut gain = 0.0_f64; let mut loss = 0.0_f64;
            for j in (i - period + 1)..=i {
                if j == 0 { continue; }
                let d = cr[j] - cr[j - 1];
                if d > 0.0 { gain += d; } else { loss += d.abs(); }
            }
            gain /= period as f64; loss /= period as f64;
            if loss == 0.0 { 100.0 } else { 100.0 - 100.0 / (1.0 + gain / loss) }
        });

        let ce = closes_arc.clone();
        eng.register_fn("ema_impl", move |period: i64| -> f64 {
            let period = period as usize;
            if period == 0 { return ce.get(i).copied().unwrap_or(0.0); }
            let k = 2.0 / (period as f64 + 1.0);
            let start = i.saturating_sub(period * 5);
            let mut e = ce[start];
            for j in (start + 1)..=i { e = ce[j] * k + e * (1.0 - k); }
            e
        });

        let cs = closes_arc.clone();
        eng.register_fn("sma_impl", move |period: i64| -> f64 {
            let period = period as usize;
            if period == 0 { return 0.0; }
            let start = if i + 1 >= period { i + 1 - period } else { 0 };
            let slice = &cs[start..=i];
            slice.iter().sum::<f64>() / slice.len() as f64
        });

        let cs_macd = closes_arc.clone();
        eng.register_fn("macd_hist_impl", move |fast: i64, slow: i64, signal: i64| -> f64 {
            macd_hist_value(&cs_macd, i, fast as usize, slow as usize, signal as usize)
        });

        let ha = highs_arc.clone(); let la = lows_arc.clone(); let ca = closes_arc.clone();
        eng.register_fn("atr_impl", move |period: i64| -> f64 {
            let period = (period.max(1)) as usize;
            if i == 0 { return 0.0; }
            let start = i.saturating_sub(period * 3);
            let tr_vals: Vec<f64> = ((start + 1)..=i).map(|j| {
                (ha[j] - la[j]).max((ha[j] - ca[j-1]).abs()).max((la[j] - ca[j-1]).abs())
            }).collect();
            if tr_vals.is_empty() { return 0.0; }
            if tr_vals.len() < period { return tr_vals.iter().sum::<f64>() / tr_vals.len() as f64; }
            let mut atr = tr_vals[..period].iter().sum::<f64>() / period as f64;
            for j in period..tr_vals.len() { atr = (atr * (period - 1) as f64 + tr_vals[j]) / period as f64; }
            atr
        });

        // No-ops for unused indicators/commands
        eng.register_fn("set_stop_loss_impl",  |_: f64| {});
        eng.register_fn("set_take_profit_impl", |_: f64| {});
        eng.register_fn("log_impl", |_: String| {});

        // ── Binary buy: bet YES (price goes UP) ──────────────────────────────
        let sb  = state.clone();
        let tp  = yes_token_price;
        let bpc = c.close;
        let bts = ts.clone();
        eng.register_fn("buy_impl", move |frac: f64| {
            let mut s = sb.lock().unwrap();
            if s.bet_active || s.balance <= 0.0 { return; }
            let stake = (s.balance * frac.max(0.0).min(1.0)).max(0.0);
            if stake == 0.0 { return; }
            s.bet_active      = true;
            s.bet_direction   = 1;
            s.bet_entry_close = bpc;
            s.bet_entry_idx   = i;
            s.bet_token_price = tp;
            s.bet_stake       = stake;
            s.balance        -= stake;
            s.sum_token_price += tp;
            tracing::debug!("[BINARY] BET YES stake=${:.2} token_price={:.3} entry={:.2}", stake, tp, bpc);
            let _ = bts.len(); // keep borrow alive
        });

        // ── Binary sell: bet NO (price goes DOWN) ────────────────────────────
        let ss  = state.clone();
        let ntp = 1.0 - yes_token_price; // NO token = complement
        let spc = c.close;
        let sts = ts.clone();
        eng.register_fn("sell_impl", move |frac: f64| {
            let mut s = ss.lock().unwrap();
            if s.bet_active || s.balance <= 0.0 { return; }
            let stake = (s.balance * frac.max(0.0).min(1.0)).max(0.0);
            if stake == 0.0 { return; }
            s.bet_active      = true;
            s.bet_direction   = -1;
            s.bet_entry_close = spc;
            s.bet_entry_idx   = i;
            s.bet_token_price = ntp.max(0.03); // never below 3¢
            s.bet_stake       = stake;
            s.balance        -= stake;
            s.sum_token_price += ntp.max(0.03);
            tracing::debug!("[BINARY] BET NO stake=${:.2} no_token_price={:.3} entry={:.2}", stake, ntp, spc);
            let _ = sts.len();
        });

        // ── Key-value store ──────────────────────────────────────────────────
        let sset = state.clone();
        eng.register_fn("set_impl", move |key: String, val: f64| {
            sset.lock().unwrap().kv.insert(key, val);
        });
        let sget = state.clone();
        eng.register_fn("get_impl", move |key: String, def: f64| -> f64 {
            sget.lock().unwrap().kv.get(&key).copied().unwrap_or(def)
        });

        // ── Run script for this candle ────────────────────────────────────────
        let full_script = format!(
            r#"
{patched}

let ctx = #{{}};
ctx.close          = {close};
ctx.open           = {open};
ctx.high           = {high};
ctx.low            = {low};
ctx.volume         = {volume};
ctx.index          = {index};
ctx.position       = {position};
ctx.entry_price    = {entry_price};
ctx.entry_index    = {entry_index};
ctx.balance        = {balance};
ctx.token_price    = {token_price};
ctx.window_minutes = {window_minutes};
ctx.open_positions = {open_pos};
on_candle(ctx);
"#,
            patched        = patched_script,
            close          = c.close,
            open           = c.open,
            high           = c.high,
            low            = c.low,
            volume         = c.volume,
            index          = i,
            position       = cur_position,
            entry_price    = cur_entry_price,
            entry_index    = cur_entry_index,
            balance        = cur_balance,
            token_price    = yes_token_price,
            window_minutes = window_candles,
            open_pos       = if bet_active { 1i64 } else { 0i64 },
        );

        if let Err(e) = eng.run(&full_script) {
            tracing::warn!("[BINARY] Script error at candle {i}: {e}");
        }

        // Track portfolio value (balance + in-flight stake)
        let snap = {
            let s = state.lock().unwrap();
            if s.bet_active { s.balance + s.bet_stake } else { s.balance }
        };
        portfolio_values.push(snap);
        if snap > peak { peak = snap; }
        let dd = if peak > 0.0 { (peak - snap) / peak * 100.0 } else { 0.0 };
        if dd > max_dd { max_dd = dd; }
    }

    // Force-resolve any still-open bet at the last candle
    if let Some(last) = candles.last() {
        let ts_last = chrono::DateTime::from_timestamp_millis(last.open_time_ms)
            .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .unwrap_or_else(|| "final".to_string());
        let mut s = state.lock().unwrap();
        if s.bet_active {
            let direction   = s.bet_direction;
            let token_price = s.bet_token_price;
            let stake       = s.bet_stake;
            let went_up     = last.close > s.bet_entry_close;
            let won         = (direction > 0 && went_up) || (direction < 0 && !went_up);
            let tokens       = stake / token_price;
            let gross_payout = if won { tokens } else { 0.0 };
            let net_payout   = gross_payout * (1.0 - fee_pct / 100.0);
            let pnl          = net_payout - stake;
            s.balance += net_payout;
            if won { s.total_correct += 1; }
            s.total_resolved += 1;
            let side_str = match (direction > 0, won) {
                (true,  true)  => "yes_win",
                (true,  false) => "yes_loss",
                (false, true)  => "no_win",
                (false, false) => "no_loss",
            };
            s.trades.push(Trade {
                timestamp: ts_last,
                side: side_str.into(),
                price: token_price, size: tokens, pnl,
                debug: None,
            });
            s.bet_active = false;
        }
    }

    // ── Compute metrics ───────────────────────────────────────────────────────
    let s = state.lock().unwrap();
    let final_balance     = s.balance;
    let total_return_pct  = (final_balance - initial_balance) / initial_balance * 100.0;
    let total_trades      = s.total_resolved;
    let wins              = s.total_correct;
    let win_rate_pct      = if total_trades > 0 { wins as f64 / total_trades as f64 * 100.0 } else { 0.0 };

    // Worst trades = 5 bets with lowest PnL
    let mut sorted = s.trades.clone();
    sorted.sort_by(|a, b| a.pnl.partial_cmp(&b.pnl).unwrap_or(std::cmp::Ordering::Equal));
    let worst_trades: Vec<WorstTrade> = sorted.iter().take(5).map(|t| WorstTrade {
        timestamp: t.timestamp.clone(), side: t.side.clone(), price: t.price, pnl: t.pnl,
    }).collect();

    // All trades for equity curve (running balance after each completed bet)
    let mut running_bal = initial_balance;
    let all_trades: Vec<AllTrade> = s.trades.iter().map(|t| {
        running_bal += t.pnl;
        AllTrade {
            timestamp: t.timestamp.clone(), side: t.side.clone(),
            price: t.price, size: t.size, pnl: t.pnl, balance: running_bal,
            debug: None,
        }
    }).collect();

    // Sharpe ratio (annualized, based on 1-minute portfolio snapshots)
    let returns: Vec<f64> = portfolio_values.windows(2)
        .map(|w| if w[0] > 0.0 { (w[1] - w[0]) / w[0] } else { 0.0 })
        .collect();
    let mean_r = returns.iter().sum::<f64>() / returns.len().max(1) as f64;
    let var_r  = returns.iter().map(|r| (r - mean_r).powi(2)).sum::<f64>() / returns.len().max(1) as f64;
    let sharpe = if var_r.sqrt() > 0.0 {
        (mean_r / var_r.sqrt()) * (252.0 * 1440.0_f64).sqrt()
    } else { 0.0 };

    // Binary-specific metrics
    let avg_token_price       = if s.total_resolved > 0 { Some(s.sum_token_price / s.total_resolved as f64) } else { None };
    let correct_direction_pct = if s.total_resolved > 0 { Some(s.total_correct as f64 / s.total_resolved as f64 * 100.0) } else { None };
    let break_even_win_rate   = avg_token_price.map(|p| p * 100.0);

    let analysis = build_binary_analysis(
        total_return_pct, win_rate_pct, total_trades,
        avg_token_price, correct_direction_pct, break_even_win_rate,
        window_candles,
    );

    Ok(BacktestMetrics {
        total_return_pct,
        sharpe_ratio: sharpe,
        max_drawdown_pct: max_dd,
        win_rate_pct,
        total_trades,
        worst_trades,
        all_trades,
        analysis,
        avg_token_price,
        correct_direction_pct,
        break_even_win_rate,
        markets_tested: None, flat_debugs: vec![],
    })
}

// ── Polymarket Slug-Aligned Binary Engine ──────────────────────────────────

/// Round `ts` up to the nearest multiple of `step`.
fn align_up_to(ts: i64, step: i64) -> i64 {
    let rem = ts % step;
    if rem == 0 { ts } else { ts + (step - rem) }
}

/// Run Polymarket BTC UPDOWN backtesting using slug-aligned market windows.
///
/// Slugs are deterministic: `btc-updown-{N}m-{window_ts}` where `window_ts % (N*60) == 0`.
/// For each window the strategy fires ONCE at the decision candle
/// (minute N-2 from window start, the last complete 1m candle before close).
/// Resolution: close of minute N-1 candle vs window open price.
///
/// Extra ctx fields: `ctx.window_open`, `ctx.window_minutes`, `ctx.token_price`.
fn run_polymarket_slug_backtest(
    script_content: String,
    candles: Vec<Candle>,
    initial_balance: f64,
    fee_pct: f64,
    window_minutes: usize,
    resolution_logic: &str,
    threshold: Option<f64>,
    max_stake_usd: Option<f64>,
    max_entry_price: Option<f64>,
    sizing_mode: &str,
    sizing_value: f64,
) -> anyhow::Result<BacktestMetrics> {
    use rhai::{Engine, Scope};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    if candles.is_empty() {
        return Err(anyhow::anyhow!("No 1m candle data for slug backtest"));
    }

    {
        let check = Engine::new();
        let ast_check = check.compile(&script_content)
            .map_err(|e| anyhow::anyhow!("Script compile error: {e}"))?;
        if !ast_check.iter_functions().any(|f| f.name == "on_candle") {
            return Err(anyhow::anyhow!("Binary slug backtest requires an on_candle(ctx) function."));
        }
    }

    // Build timestamp-seconds → candle-index lookup
    let ts_to_idx: HashMap<i64, usize> = candles.iter().enumerate()
        .map(|(idx, c)| (c.open_time_ms / 1000, idx))
        .collect();

    let window_secs  = (window_minutes as i64) * 60;
    let first_ts     = candles[0].open_time_ms / 1000;
    let last_ts      = candles.last().unwrap().open_time_ms / 1000;
    let first_window = align_up_to(first_ts, window_secs);

    let closes:  Vec<f64> = candles.iter().map(|c| c.close).collect();
    let highs:   Vec<f64> = candles.iter().map(|c| c.high).collect();
    let lows:    Vec<f64> = candles.iter().map(|c| c.low).collect();
    let volumes: Vec<f64> = candles.iter().map(|c| c.volume).collect();
    let closes_arc  = Arc::new(closes);
    let highs_arc   = Arc::new(highs);
    let lows_arc    = Arc::new(lows);
    let volumes_arc = Arc::new(volumes);

    let patched_script = script_content
        .replace("ctx.rsi(",             "rsi_impl(")
        .replace("ctx.ema(",             "ema_impl(")
        .replace("ctx.atr(",             "atr_impl(")
        .replace("ctx.sma(",             "sma_impl(")
        .replace("ctx.macd_hist(",       "macd_hist_impl(")
        .replace("ctx.close_at(",        "close_at_impl(")
        .replace("ctx.high_at(",         "high_at_impl(")
        .replace("ctx.low_at(",          "low_at_impl(")
        .replace("ctx.volume_at(",       "volume_at_impl(")
        .replace("ctx.set_stop_loss(",   "set_stop_loss_impl(")
        .replace("ctx.set_take_profit(", "set_take_profit_impl(")
        .replace("ctx.buy(",             "buy_impl(")
        .replace("ctx.sell(",            "sell_impl(")
        .replace("ctx.short(",           "sell_impl(")
        .replace("ctx.set(",             "set_impl(")
        .replace("ctx.get(",             "get_impl(")
        .replace("ctx.log(",             "log_impl(");

    #[derive(Clone)]
    struct SlugState {
        balance:         f64,
        trades:          Vec<Trade>,
        kv:              std::collections::HashMap<String, f64>,
        total_correct:   u32,
        total_resolved:  u32,
        sum_token_price: f64,
        pending_buy:     bool,
        pending_sell:    bool,
    }

    let state = Arc::new(Mutex::new(SlugState {
        balance: initial_balance, trades: Vec::new(),
        kv: std::collections::HashMap::new(),
        total_correct: 0, total_resolved: 0, sum_token_price: 0.0,
        pending_buy: false, pending_sell: false,
    }));

    let mut portfolio_values: Vec<f64> = vec![initial_balance];
    let mut peak   = initial_balance;
    let mut max_dd = 0.0_f64;
    let mut markets_tested: u32 = 0;
    let mut flat_debugs: Vec<(String, std::collections::HashMap<String, f64>)> = Vec::new();

    // ── Build engine ONCE — compile AST once, reuse across all windows ────────
    let cur_idx = Arc::new(Mutex::new(0usize));

    let mut engine = Engine::new();
    engine.set_max_operations(500_000);
    engine.set_max_call_levels(64);

    let cl = closes_arc.clone();
    engine.register_fn("close_at_impl",  move |idx: i64| -> f64 { cl.get(idx as usize).copied().unwrap_or(0.0) });
    let vl = volumes_arc.clone();
    engine.register_fn("volume_at_impl", move |idx: i64| -> f64 { vl.get(idx as usize).copied().unwrap_or(0.0) });
    let hl = highs_arc.clone();
    engine.register_fn("high_at_impl",   move |idx: i64| -> f64 { hl.get(idx as usize).copied().unwrap_or(0.0) });
    let ll = lows_arc.clone();
    engine.register_fn("low_at_impl",    move |idx: i64| -> f64 { ll.get(idx as usize).copied().unwrap_or(0.0) });

    let ci = cur_idx.clone(); let cr = closes_arc.clone();
    engine.register_fn("rsi_impl", move |period: i64| -> f64 {
        let i = *ci.lock().unwrap();
        let period = period as usize;
        if i < period { return 50.0; }
        let mut gain = 0.0_f64; let mut loss = 0.0_f64;
        for j in (i - period + 1)..=i {
            if j == 0 { continue; }
            let d = cr[j] - cr[j - 1];
            if d > 0.0 { gain += d; } else { loss += d.abs(); }
        }
        gain /= period as f64; loss /= period as f64;
        if loss == 0.0 { 100.0 } else { 100.0 - 100.0 / (1.0 + gain / loss) }
    });

    let ci = cur_idx.clone(); let ce = closes_arc.clone();
    engine.register_fn("ema_impl", move |period: i64| -> f64 {
        let i = *ci.lock().unwrap();
        let period = period as usize;
        if period == 0 { return ce.get(i).copied().unwrap_or(0.0); }
        let k = 2.0 / (period as f64 + 1.0);
        let start = i.saturating_sub(period * 5);
        let mut e = ce[start];
        for j in (start + 1)..=i { e = ce[j] * k + e * (1.0 - k); }
        e
    });

    let ci = cur_idx.clone(); let cs = closes_arc.clone();
    engine.register_fn("sma_impl", move |period: i64| -> f64 {
        let i = *ci.lock().unwrap();
        let period = period as usize;
        if period == 0 { return 0.0; }
        let start = if i + 1 >= period { i + 1 - period } else { 0 };
        cs[start..=i].iter().sum::<f64>() / (i - start + 1) as f64
    });

    let ci_macd = cur_idx.clone(); let ce_macd = closes_arc.clone();
    engine.register_fn("macd_hist_impl", move |fast: i64, slow: i64, signal: i64| -> f64 {
        let i = *ci_macd.lock().unwrap();
        macd_hist_value(&ce_macd, i, fast as usize, slow as usize, signal as usize)
    });

    let ci = cur_idx.clone();
    let ha = highs_arc.clone(); let la = lows_arc.clone(); let ca = closes_arc.clone();
    engine.register_fn("atr_impl", move |period: i64| -> f64 {
        let i = *ci.lock().unwrap();
        let period = (period.max(1)) as usize;
        if i == 0 { return 0.0; }
        let start = i.saturating_sub(period * 3);
        let tr_vals: Vec<f64> = ((start + 1)..=i).map(|j| {
            (ha[j] - la[j]).max((ha[j] - ca[j-1]).abs()).max((la[j] - ca[j-1]).abs())
        }).collect();
        if tr_vals.is_empty() { return 0.0; }
        if tr_vals.len() < period { return tr_vals.iter().sum::<f64>() / tr_vals.len() as f64; }
        let mut atr = tr_vals[..period].iter().sum::<f64>() / period as f64;
        for j in period..tr_vals.len() { atr = (atr * (period - 1) as f64 + tr_vals[j]) / period as f64; }
        atr
    });

    engine.register_fn("set_stop_loss_impl",   |_: f64| {});
    engine.register_fn("set_take_profit_impl", |_: f64| {});
    engine.register_fn("log_impl", |_msg: rhai::Dynamic| {});

    let sb = state.clone();
    engine.register_fn("buy_impl", move |_frac: f64| {
        let mut s = sb.lock().unwrap();
        if !s.pending_buy && !s.pending_sell && s.balance > 0.0 { s.pending_buy = true; }
    });
    let ss = state.clone();
    engine.register_fn("sell_impl", move |_frac: f64| {
        let mut s = ss.lock().unwrap();
        if !s.pending_buy && !s.pending_sell && s.balance > 0.0 { s.pending_sell = true; }
    });
    let sset = state.clone();
    engine.register_fn("set_impl", move |key: String, val: f64| {
        sset.lock().unwrap().kv.insert(key, val);
    });
    let sget = state.clone();
    engine.register_fn("get_impl", move |key: String, def: f64| -> f64 {
        sget.lock().unwrap().kv.get(&key).copied().unwrap_or(def)
    });

    let ast = engine.compile(&patched_script)
        .map_err(|e| anyhow::anyhow!("Script compile error (patched): {e}"))?;
    // ─────────────────────────────────────────────────────────────────────────

    let mut window_ts = first_window;
    while window_ts + window_secs <= last_ts {
        // For a 5m window: minute0=T, decision=T+180s, resolution=T+240s
        let minute0_ts    = window_ts;
        let decision_ts   = window_ts + ((window_minutes as i64) - 2) * 60;
        let resolution_ts = window_ts + ((window_minutes as i64) - 1) * 60;
        window_ts += window_secs;

        let (Some(&m0_idx), Some(&dec_idx), Some(&res_idx)) = (
            ts_to_idx.get(&minute0_ts),
            ts_to_idx.get(&decision_ts),
            ts_to_idx.get(&resolution_ts),
        ) else { continue; };

        let window_open       = candles[m0_idx].open;
        let dec               = &candles[dec_idx];
        let res_close         = candles[res_idx].close;
        // Resolution: use logic from the series definition
        let went_up = match resolution_logic {
            "threshold_above" => res_close > threshold.unwrap_or(f64::MAX),
            "threshold_below" => res_close < threshold.unwrap_or(f64::MIN),
            _                 => res_close > window_open,   // default: price_up
        };
        let resolution_value = res_close;
        let thr_val = threshold.unwrap_or(0.0);
        // Token price: for price markets use momentum model; for threshold markets use flat 0.50
        let yes_token_price = match resolution_logic {
            "threshold_above" | "threshold_below" => 0.50_f64,
            _ => {
                let delta_abs = if window_open > 0.0 {
                    ((dec.close - window_open) / window_open * 100.0).abs()
                } else { 0.0 };
                polymarket_token_price(delta_abs)
            }
        };
        markets_tested += 1;

        {
            let mut s = state.lock().unwrap();
            s.pending_buy = false; s.pending_sell = false;
            // Strip stale debug_* from kv so an early-return doesn’t leak
            // previous-window indicators into the current trade/flat_debug.
            s.kv.retain(|k, _| !k.starts_with("debug_"));
        }

        let (cur_balance, _cur_position) = { let s = state.lock().unwrap(); (s.balance, 0.0f64) };

        let ts_str = chrono::DateTime::from_timestamp_millis(dec.open_time_ms)
            .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .unwrap_or_else(|| dec.open_time_ms.to_string());

        // Update shared index so indicator closures see the current decision candle
        *cur_idx.lock().unwrap() = dec_idx;

        let mut ctx_map = rhai::Map::new();
        ctx_map.insert("close".into(),          rhai::Dynamic::from(dec.close));
        ctx_map.insert("open".into(),            rhai::Dynamic::from(dec.open));
        ctx_map.insert("high".into(),            rhai::Dynamic::from(dec.high));
        ctx_map.insert("low".into(),             rhai::Dynamic::from(dec.low));
        ctx_map.insert("volume".into(),          rhai::Dynamic::from(dec.volume));
        ctx_map.insert("index".into(),           rhai::Dynamic::from(dec_idx as i64));
        ctx_map.insert("position".into(),        rhai::Dynamic::from(0.0_f64));
        ctx_map.insert("entry_price".into(),     rhai::Dynamic::from(0.0_f64));
        ctx_map.insert("entry_index".into(),     rhai::Dynamic::from(0i64));
        ctx_map.insert("balance".into(),         rhai::Dynamic::from(cur_balance));
        ctx_map.insert("open_positions".into(),  rhai::Dynamic::from(0i64));
        ctx_map.insert("window_open".into(),       rhai::Dynamic::from(window_open));
        ctx_map.insert("window_minutes".into(),    rhai::Dynamic::from(window_minutes as i64));
        ctx_map.insert("token_price".into(),       rhai::Dynamic::from(yes_token_price));
        ctx_map.insert("threshold".into(),         rhai::Dynamic::from(thr_val));
        ctx_map.insert("resolution_value".into(),  rhai::Dynamic::from(resolution_value));
        // ctx.value is an alias for ctx.close (more semantic for non-price markets)
        ctx_map.insert("value".into(),             rhai::Dynamic::from(dec.close));

        let mut scope = Scope::new();
        if let Err(e) = engine.call_fn::<()>(&mut scope, &ast, "on_candle", (rhai::Dynamic::from_map(ctx_map),)) {
            tracing::warn!("[SLUG] Script error at {ts_str}: {e}");
        }

        let (pb, ps, bal) = {
            let s = state.lock().unwrap();
            (s.pending_buy, s.pending_sell, s.balance)
        };

        if pb || ps {
            let bet_up  = pb;
            let raw_stake = if sizing_mode == "fixed" {
                sizing_value.min(bal).max(0.0)
            } else {
                (bal * sizing_value.max(0.0).min(1.0)).max(0.0)
            };
            // Enforce Polymarket position limit: stake cannot exceed available market liquidity.
            // Real 5-min binary markets typically have $500-$3,000 USDC of liquidity per window.
            let stake = if let Some(max_s) = max_stake_usd {
                raw_stake.min(max_s)
            } else {
                raw_stake
            };
            // Also enforce minimum order size ($5 USDC per Polymarket API)
            if stake < 5.0 { continue; }
            let token_p = if bet_up { yes_token_price } else { (1.0 - yes_token_price).max(0.03) };
            // Skip if token price exceeds max entry price threshold
            if let Some(max_ep) = max_entry_price {
                if token_p > max_ep {
                    continue;
                }
            }
            let won     = (bet_up && went_up) || (!bet_up && !went_up);
            let tokens  = if token_p > 0.0 { stake / token_p } else { 0.0 };
            let net_pay = if won { tokens * (1.0 - fee_pct / 100.0) } else { 0.0 };
            let pnl     = net_pay - stake;

            let side_str = match (bet_up, won) {
                (true,  true)  => "yes_win",
                (true,  false) => "yes_loss",
                (false, true)  => "no_win",
                (false, false) => "no_loss",
            };

            let mut s = state.lock().unwrap();
            s.balance -= stake;
            s.balance += net_pay;
            if won { s.total_correct += 1; }
            s.total_resolved += 1;
            s.sum_token_price += token_p;
            let debug = s.kv.clone();
            s.trades.push(Trade { timestamp: ts_str, side: side_str.into(), price: token_p, size: tokens, pnl, debug: Some(debug) });
        } else {
            let s = state.lock().unwrap();
            let kv = s.kv.clone();
            drop(s);
            if !kv.is_empty() {
                flat_debugs.push((ts_str, kv));
            }
        }

        let snap = state.lock().unwrap().balance;
        portfolio_values.push(snap);
        if snap > peak { peak = snap; }
        let dd = if peak > 0.0 { (peak - snap) / peak * 100.0 } else { 0.0 };
        if dd > max_dd { max_dd = dd; }
    }

    let s = state.lock().unwrap();
    let final_balance    = s.balance;
    let total_return_pct = (final_balance - initial_balance) / initial_balance * 100.0;
    let total_trades     = s.total_resolved;
    let wins             = s.total_correct;
    let win_rate_pct     = if total_trades > 0 { wins as f64 / total_trades as f64 * 100.0 } else { 0.0 };

    let mut sorted = s.trades.clone();
    sorted.sort_by(|a, b| a.pnl.partial_cmp(&b.pnl).unwrap_or(std::cmp::Ordering::Equal));
    let worst_trades: Vec<WorstTrade> = sorted.iter().take(5).map(|t| WorstTrade {
        timestamp: t.timestamp.clone(), side: t.side.clone(), price: t.price, pnl: t.pnl,
    }).collect();

    let mut running_bal = initial_balance;
    let all_trades: Vec<AllTrade> = s.trades.iter().map(|t| {
        running_bal += t.pnl;
        AllTrade { timestamp: t.timestamp.clone(), side: t.side.clone(), price: t.price, size: t.size, pnl: t.pnl, balance: running_bal, debug: t.debug.clone() }
    }).collect();

    let returns: Vec<f64> = portfolio_values.windows(2)
        .map(|w| if w[0] > 0.0 { (w[1] - w[0]) / w[0] } else { 0.0 }).collect();
    let mean_r = if returns.is_empty() { 0.0 } else { returns.iter().sum::<f64>() / returns.len() as f64 };
    let var_r  = if returns.is_empty() { 0.0 } else { returns.iter().map(|r| (r - mean_r).powi(2)).sum::<f64>() / returns.len() as f64 };
    let sharpe = if var_r.sqrt() > 0.0 { (mean_r / var_r.sqrt()) * (252.0 * 1440.0_f64).sqrt() } else { 0.0 };

    let avg_token_price       = if s.total_resolved > 0 { Some(s.sum_token_price / s.total_resolved as f64) } else { None };
    let correct_direction_pct = if s.total_resolved > 0 { Some(s.total_correct as f64 / s.total_resolved as f64 * 100.0) } else { None };
    let break_even_win_rate   = avg_token_price.map(|p| p * 100.0);
    let dm                    = window_minutes.saturating_sub(1);

    let base_analysis = build_binary_analysis(
        total_return_pct, win_rate_pct, total_trades,
        avg_token_price, correct_direction_pct, break_even_win_rate,
        window_minutes,
    );
    let analysis = format!(
        "[{markets_tested} markets btc-updown-{window_minutes}m \u{00b7} decision @ minuto {dm}] {base_analysis}"
    );

    Ok(BacktestMetrics {
        total_return_pct,
        sharpe_ratio: sharpe,
        max_drawdown_pct: max_dd,
        win_rate_pct,
        total_trades,
        worst_trades,
        all_trades,
        analysis,
        avg_token_price,
        correct_direction_pct,
        break_even_win_rate,
        markets_tested: Some(markets_tested),
        flat_debugs,
    })
}

// ── Entry point ──────────────────────────────────────────────────────────────

/// Entry point called by BacktestRunTool: fetches candles and runs the real engine.
/// Falls back to the deterministic stub if the fetch or execution fails.
pub async fn run_backtest_engine(
    script_path: &std::path::Path,
    market_type: &str,
    symbol: &str,
    interval: &str,
    from_date: &str,
    to_date: &str,
    initial_balance: f64,
    fee_pct: f64,
    resolution_logic: &str,
    threshold: Option<f64>,
    max_position_usd: Option<f64>,
    max_entry_price: Option<f64>,
    sizing_mode: &str,
    sizing_value: f64,
    workspace_dir: &std::path::Path,
) -> BacktestMetrics {
    tracing::info!(
        "[BACKTEST] Starting backtest: script={}, market={}, symbol={}, interval={}, from={}, to={}, balance={}, fee={}%",
        script_path.display(), market_type, symbol, interval, from_date, to_date, initial_balance, fee_pct
    );

    let script_content = match std::fs::read_to_string(script_path) {
        Ok(s) => {
            tracing::debug!("[BACKTEST] Script loaded, {} bytes, first 200 chars: {:?}",
                s.len(), &s.chars().take(200).collect::<String>());
            s
        }
        Err(e) => {
            tracing::error!("[BACKTEST] Failed to read script: {e}");
            return BacktestMetrics {
                total_return_pct: 0.0, sharpe_ratio: 0.0, max_drawdown_pct: 0.0,
                win_rate_pct: 0.0, total_trades: 0, worst_trades: vec![], all_trades: vec![],
                avg_token_price: None, correct_direction_pct: None, break_even_win_rate: None,
                markets_tested: None, flat_debugs: vec![],
                analysis: format!("Error reading script: {e}"),
            };
        }
    };

    // ── polymarket_binary: slug-aligned binary engine ────────────────────────────────
    // Data source determined by resolution_logic:
    //   price_up      -> Binance 1m candles
    //   threshold_*   -> Open-Meteo daily weather
    if market_type == "polymarket_binary" {
        let window_minutes = parse_interval_to_minutes(interval);
        let rl = resolution_logic.to_string();
        let thr = threshold;

        let candles = if rl == "threshold_above" || rl == "threshold_below" {
            tracing::info!("[BACKTEST] Binary slug (Open-Meteo): daily weather for '{symbol}' ({from_date}->{to_date})...");
            match fetch_weather_candles(symbol, from_date, to_date).await {
                Ok(c) if !c.is_empty() => { tracing::info!("[BACKTEST] Fetched {} daily weather candles", c.len()); c }
                Ok(_) => return BacktestMetrics {
                    total_return_pct: 0.0, sharpe_ratio: 0.0, max_drawdown_pct: 0.0,
                    win_rate_pct: 0.0, total_trades: 0, worst_trades: vec![], all_trades: vec![],
                    avg_token_price: None, correct_direction_pct: None, break_even_win_rate: None, markets_tested: None, flat_debugs: vec![],
                    analysis: format!("No weather data from Open-Meteo for '{symbol}' ({from_date}->{to_date}). Check city name."),
                },
                Err(e) => return BacktestMetrics {
                    total_return_pct: 0.0, sharpe_ratio: 0.0, max_drawdown_pct: 0.0,
                    win_rate_pct: 0.0, total_trades: 0, worst_trades: vec![], all_trades: vec![],
                    avg_token_price: None, correct_direction_pct: None, break_even_win_rate: None, markets_tested: None, flat_debugs: vec![],
                    analysis: format!("Failed to fetch Open-Meteo data: {e}"),
                },
            }
        } else {
            tracing::info!("[BACKTEST] Binary slug (Binance): fetching 1m {symbol} candles for {window_minutes}-min windows...");
            match fetch_candles(symbol, "1m", from_date, to_date, workspace_dir).await {
                Ok(c) if !c.is_empty() => { tracing::info!("[BACKTEST] Fetched {} 1m candles", c.len()); c }
                Ok(_) => return BacktestMetrics {
                    total_return_pct: 0.0, sharpe_ratio: 0.0, max_drawdown_pct: 0.0,
                    win_rate_pct: 0.0, total_trades: 0, worst_trades: vec![], all_trades: vec![],
                    avg_token_price: None, correct_direction_pct: None, break_even_win_rate: None, markets_tested: None, flat_debugs: vec![],
                    analysis: format!("No 1m candle data from Binance for {symbol} ({from_date}->{to_date}). Check symbol."),
                },
                Err(e) => return BacktestMetrics {
                    total_return_pct: 0.0, sharpe_ratio: 0.0, max_drawdown_pct: 0.0,
                    win_rate_pct: 0.0, total_trades: 0, worst_trades: vec![], all_trades: vec![],
                    avg_token_price: None, correct_direction_pct: None, break_even_win_rate: None, markets_tested: None, flat_debugs: vec![],
                    analysis: format!("Failed to fetch Binance data: {e}"),
                },
            }
        };

        let script_for_log = script_content.clone();
        let sizing_mode_owned = sizing_mode.to_string();

        return match tokio::task::spawn_blocking(move || {
            run_polymarket_slug_backtest(script_content, candles, initial_balance, fee_pct, window_minutes, &rl, thr, max_position_usd, max_entry_price, &sizing_mode_owned, sizing_value)
        })
        .await
        {
            Ok(Ok(metrics)) => {
                tracing::info!(
                    "[BINARY-SLUG] Done: return={:.2}%, trades={}, win={:.1}%, markets={:?}",
                    metrics.total_return_pct, metrics.total_trades,
                    metrics.win_rate_pct, metrics.markets_tested
                );
                metrics
            }
            Ok(Err(e)) => {
                tracing::error!("[BINARY-SLUG] Engine error: {e}");
                tracing::debug!("[BINARY-SLUG] Script:\n{}", script_for_log);
                BacktestMetrics {
                    total_return_pct: 0.0, sharpe_ratio: 0.0, max_drawdown_pct: 0.0,
                    win_rate_pct: 0.0, total_trades: 0, worst_trades: vec![], all_trades: vec![],
                    avg_token_price: None, correct_direction_pct: None, break_even_win_rate: None,
                    markets_tested: None, flat_debugs: vec![],
                    analysis: format!("Binary slug engine error: {e}"),
                }
            }
            Err(e) => BacktestMetrics {
                total_return_pct: 0.0, sharpe_ratio: 0.0, max_drawdown_pct: 0.0,
                win_rate_pct: 0.0, total_trades: 0, worst_trades: vec![], all_trades: vec![],
                avg_token_price: None, correct_direction_pct: None, break_even_win_rate: None,
                markets_tested: None, flat_debugs: vec![],
                analysis: format!("Binary slug task panicked: {e}"),
            },
        };
    }

    // ── Standard crypto / polymarket-CLOB path ────────────────────────────────
    let data_source = if market_type == "polymarket" { "Polymarket" } else { "Binance" };
    tracing::info!("[BACKTEST] Fetching {interval} candles from {data_source} for {symbol}...");

    let candles = if market_type == "polymarket" {
        match fetch_polymarket_candles(symbol, interval, from_date, to_date, workspace_dir).await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("[BACKTEST] Polymarket fetch failed: {e}");
                return BacktestMetrics {
                    total_return_pct: 0.0, sharpe_ratio: 0.0, max_drawdown_pct: 0.0,
                    win_rate_pct: 0.0, total_trades: 0, worst_trades: vec![], all_trades: vec![],
                    avg_token_price: None, correct_direction_pct: None, break_even_win_rate: None,
                    markets_tested: None, flat_debugs: vec![],
                    analysis: format!(
                        "Could not fetch historical data from Polymarket: {e}. \
                        Ensure the condition ID is valid."
                    ),
                };
            }
        }
    } else {
        match fetch_candles(symbol, interval, from_date, to_date, workspace_dir).await {
            Ok(c) if !c.is_empty() => {
                tracing::info!("[BACKTEST] Fetched {} candles from Binance", c.len());
                if let Some(first) = c.first() {
                    tracing::debug!("[BACKTEST] First candle: open={}, close={}, vol={}",
                        first.open, first.close, first.volume);
                }
                c
            }
            Ok(_) => {
                tracing::warn!("[BACKTEST] Binance returned empty candle data for {symbol}");
                return BacktestMetrics {
                    total_return_pct: 0.0, sharpe_ratio: 0.0, max_drawdown_pct: 0.0,
                    win_rate_pct: 0.0, total_trades: 0, worst_trades: vec![], all_trades: vec![],
                    avg_token_price: None, correct_direction_pct: None, break_even_win_rate: None,
                    analysis: format!(
                        "No candle data returned from Binance for {symbol} ({from_date}→{to_date}). \
                        Check the symbol name and date range."
                    ),
                    markets_tested: None, flat_debugs: vec![],
                };
            }
            Err(e) => {
                tracing::error!("[BACKTEST] Binance fetch failed: {e}");
                return BacktestMetrics {
                    total_return_pct: 0.0, sharpe_ratio: 0.0, max_drawdown_pct: 0.0,
                    win_rate_pct: 0.0, total_trades: 0, worst_trades: vec![], all_trades: vec![],
                    avg_token_price: None, correct_direction_pct: None, break_even_win_rate: None,
                    markets_tested: None, flat_debugs: vec![],
                    analysis: format!(
                        "Could not fetch historical data from Binance: {e}. \
                        Ensure the gateway has internet access."
                    ),
                };
            }
        }
    };

    let num_candles = candles.len();
    let interval_for_log = interval.to_string();
    let data_source_for_log = data_source.to_string();
    let script_for_log = script_content.clone();
    let sizing_mode_owned = sizing_mode.to_string();

    // Run Rhai engine in blocking thread (CPU-bound)
    tracing::info!("[BACKTEST] Running Rhai engine on {} candles...", num_candles);
    match tokio::task::spawn_blocking(move || {
        run_rhai_backtest(script_content, candles, initial_balance, fee_pct, max_entry_price, &sizing_mode_owned, sizing_value)
    })
    .await
    {
        Ok(Ok(mut metrics)) => {
            tracing::info!(
                "[BACKTEST] Completed: return={:.2}%, sharpe={:.2}, trades={}, win_rate={:.1}%",
                metrics.total_return_pct, metrics.sharpe_ratio,
                metrics.total_trades, metrics.win_rate_pct
            );
            metrics.analysis = format!(
                "[{num_candles} {interval_for_log} candles from {data_source_for_log}] {}",
                metrics.analysis
            );
            metrics
        }
        Ok(Err(e)) => {
            tracing::error!("[BACKTEST] Rhai execution error: {e}");
            tracing::debug!("[BACKTEST] Failed script content:\n{}", script_for_log);
            BacktestMetrics {
                total_return_pct: 0.0, sharpe_ratio: 0.0, max_drawdown_pct: 0.0,
                win_rate_pct: 0.0, total_trades: 0, worst_trades: vec![], all_trades: vec![],
                avg_token_price: None, correct_direction_pct: None, break_even_win_rate: None,
                markets_tested: None, flat_debugs: vec![],
                analysis: format!("Rhai execution error: {e}"),
            }
        }
        Err(e) => {
            tracing::error!("[BACKTEST] Backtest task panicked: {e}");
            BacktestMetrics {
                total_return_pct: 0.0, sharpe_ratio: 0.0, max_drawdown_pct: 0.0,
                win_rate_pct: 0.0, total_trades: 0, worst_trades: vec![], all_trades: vec![],
                avg_token_price: None, correct_direction_pct: None, break_even_win_rate: None,
                markets_tested: None, flat_debugs: vec![],
                analysis: format!("Backtest task panicked: {e}"),
            }
        }
    }
}


// ── Live trading helpers ───────────────────────────────────────────────────

/// Run the Rhai strategy on a pre-built candle buffer (no REST fetch).
/// Used by the live strategy runner after each WebSocket candle arrives.
/// The buffer is a rolling window; trim it before calling.
pub fn run_rhai_on_candle_buffer(
    script_content: &str,
    candles: Vec<Candle>,
    initial_balance: f64,
    fee_pct: f64,
) -> BacktestMetrics {
    run_rhai_backtest(script_content.to_string(), candles, initial_balance, fee_pct, None, "percent", 1.0)
        .unwrap_or_else(|e| BacktestMetrics {
            total_return_pct: 0.0, sharpe_ratio: 0.0, max_drawdown_pct: 0.0,
            win_rate_pct: 0.0, total_trades: 0, worst_trades: vec![], all_trades: vec![],
            avg_token_price: None, correct_direction_pct: None, break_even_win_rate: None,
            markets_tested: None, flat_debugs: vec![],
            analysis: format!("Strategy error: {e}"),
        })
}

/// Fetch the last `limit` closed candles from Binance REST (no disk cache).
/// Used for live strategy warmup before the WebSocket feed connects.
pub async fn fetch_recent_candles(
    symbol: &str,
    interval: &str,
    limit: usize,
) -> anyhow::Result<Vec<Candle>> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://api.binance.com/api/v3/klines?symbol={}&interval={}&limit={}",
        symbol.to_uppercase(), interval, limit.min(1000)
    );
    tracing::info!("[LIVE] Warmup fetch: {url}");
    let rows: Vec<serde_json::Value> = client.get(&url)
        .send().await?
        .error_for_status()?
        .json().await?;
    let candles: Vec<Candle> = rows.iter().filter_map(|row| {
        let arr = row.as_array()?;
        Some(Candle {
            open_time_ms: arr[0].as_i64()?,
            open:   arr[1].as_str()?.parse().ok()?,
            high:   arr[2].as_str()?.parse().ok()?,
            low:    arr[3].as_str()?.parse().ok()?,
            close:  arr[4].as_str()?.parse().ok()?,
            volume: arr[5].as_str()?.parse().ok()?,
        })
    }).collect();
    tracing::info!("[LIVE] Warmup: {} candles fetched for {symbol} {interval}", candles.len());
    Ok(candles)
}


/// Result of evaluating a live trading signal.
pub struct LiveSignalResult {
    pub signal: String,
    pub size: f64,
    pub debug: std::collections::HashMap<String, f64>,
    /// Updated kv state after the call — persist across windows to carry avg_vol etc.
    pub kv_state: std::collections::HashMap<String, f64>,
}

/// Evaluate the live trading signal for the *current* (incomplete) window.
/// Runs the Rhai script on the decision candle and returns "yes", "no", or "flat".
/// This does NOT compute P&L — it only extracts the directional signal.
pub fn run_polymarket_live_signal(
    script_content: &str,
    candles: Vec<Candle>,
    window_minutes: usize,
    decision_minute: Option<i64>,
    _yes_token_price: f64, // real CLOB price kept for API compatibility; strategy uses momentum model
    kv_seed: &std::collections::HashMap<String, f64>,
) -> anyhow::Result<LiveSignalResult> {
    use rhai::{Engine, Scope};
    use std::sync::{Arc, Mutex};

    if candles.is_empty() {
        return Err(anyhow::anyhow!("No candles for live signal"));
    }

    let check = Engine::new();
    let ast_check = check.compile(script_content)
        .map_err(|e| anyhow::anyhow!("Script compile error: {e}"))?;
    if !ast_check.iter_functions().any(|f| f.name == "on_candle") {
        return Err(anyhow::anyhow!("Binary strategy requires an on_candle(ctx) function."));
    }

    let window_secs = (window_minutes as i64) * 60;
    let last_candle = candles.last().unwrap();
    let last_close_ts = (last_candle.open_time_ms / 1000) + 60;
    let current_window = last_close_ts - (last_close_ts % window_secs);
    let decision_min = decision_minute.unwrap_or((window_minutes as i64) - 1);
    let decision_ts = current_window + decision_min * 60;

    // Find decision candle in buffer
    let dec_idx = candles.iter()
        .position(|c| c.open_time_ms / 1000 == decision_ts)
        .or_else(|| {
            // Fallback: closest candle before or at decision time
            candles.iter().enumerate()
                .filter(|(_, c)| (c.open_time_ms / 1000) <= decision_ts)
                .map(|(i, _)| i)
                .last()
        })
        .ok_or_else(|| anyhow::anyhow!("Decision candle not found in buffer"))?;

    let dec = &candles[dec_idx];
    let window_open = candles.iter()
        .find(|c| c.open_time_ms / 1000 == current_window)
        .map(|c| c.open)
        .unwrap_or(dec.open);

    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
    let highs: Vec<f64> = candles.iter().map(|c| c.high).collect();
    let lows: Vec<f64> = candles.iter().map(|c| c.low).collect();
    let volumes: Vec<f64> = candles.iter().map(|c| c.volume).collect();
    let closes_arc = Arc::new(closes);
    let highs_arc = Arc::new(highs);
    let lows_arc = Arc::new(lows);
    let volumes_arc = Arc::new(volumes);

    let patched_script = script_content
        .replace("ctx.rsi(",             "rsi_impl(")
        .replace("ctx.ema(",             "ema_impl(")
        .replace("ctx.atr(",             "atr_impl(")
        .replace("ctx.sma(",             "sma_impl(")
        .replace("ctx.macd_hist(",       "macd_hist_impl(")
        .replace("ctx.close_at(",        "close_at_impl(")
        .replace("ctx.high_at(",         "high_at_impl(")
        .replace("ctx.low_at(",          "low_at_impl(")
        .replace("ctx.volume_at(",       "volume_at_impl(")
        .replace("ctx.set_stop_loss(",   "set_stop_loss_impl(")
        .replace("ctx.set_take_profit(", "set_take_profit_impl(")
        .replace("ctx.buy(",             "buy_impl(")
        .replace("ctx.sell(",            "sell_impl(")
        .replace("ctx.short(",           "sell_impl(")
        .replace("ctx.set(",             "set_impl(")
        .replace("ctx.get(",             "get_impl(")
        .replace("ctx.log(",             "log_impl(");

    #[derive(Clone)]
    struct LiveState {
        pending_buy: bool,
        pending_sell: bool,
    }
    let state = Arc::new(Mutex::new(LiveState { pending_buy: false, pending_sell: false }));

    let mut engine = Engine::new();
    engine.set_max_operations(500_000);
    engine.set_max_call_levels(64);

    let cl = closes_arc.clone();
    engine.register_fn("close_at_impl",  move |idx: i64| -> f64 { cl.get(idx as usize).copied().unwrap_or(0.0) });
    let vl = volumes_arc.clone();
    engine.register_fn("volume_at_impl", move |idx: i64| -> f64 { vl.get(idx as usize).copied().unwrap_or(0.0) });
    let hl = highs_arc.clone();
    engine.register_fn("high_at_impl",   move |idx: i64| -> f64 { hl.get(idx as usize).copied().unwrap_or(0.0) });
    let ll = lows_arc.clone();
    engine.register_fn("low_at_impl",    move |idx: i64| -> f64 { ll.get(idx as usize).copied().unwrap_or(0.0) });

    let ci = Arc::new(Mutex::new(dec_idx));
    let cr = closes_arc.clone();
    engine.register_fn("rsi_impl", move |period: i64| -> f64 {
        let i = *ci.lock().unwrap();
        let period = period as usize;
        if i < period { return 50.0; }
        let mut gain = 0.0_f64; let mut loss = 0.0_f64;
        for j in (i - period + 1)..=i {
            if j == 0 { continue; }
            let d = cr[j] - cr[j - 1];
            if d > 0.0 { gain += d; } else { loss += d.abs(); }
        }
        gain /= period as f64; loss /= period as f64;
        if loss == 0.0 { 100.0 } else { 100.0 - 100.0 / (1.0 + gain / loss) }
    });

    let ci = Arc::new(Mutex::new(dec_idx));
    let ce = closes_arc.clone();
    engine.register_fn("ema_impl", move |period: i64| -> f64 {
        let i = *ci.lock().unwrap();
        let period = period as usize;
        if period == 0 { return ce.get(i).copied().unwrap_or(0.0); }
        let k = 2.0 / (period as f64 + 1.0);
        let start = i.saturating_sub(period * 5);
        let mut e = ce[start];
        for j in (start + 1)..=i { e = ce[j] * k + e * (1.0 - k); }
        e
    });

    let ci = Arc::new(Mutex::new(dec_idx));
    let cs = closes_arc.clone();
    engine.register_fn("sma_impl", move |period: i64| -> f64 {
        let i = *ci.lock().unwrap();
        let period = period as usize;
        if period == 0 { return 0.0; }
        let start = if i + 1 >= period { i + 1 - period } else { 0 };
        cs[start..=i].iter().sum::<f64>() / (i - start + 1) as f64
    });

    let ci_macd = Arc::new(Mutex::new(dec_idx));
    let ce_macd = closes_arc.clone();
    engine.register_fn("macd_hist_impl", move |fast: i64, slow: i64, signal: i64| -> f64 {
        let i = *ci_macd.lock().unwrap();
        macd_hist_value(&ce_macd, i, fast as usize, slow as usize, signal as usize)
    });

    let ci = Arc::new(Mutex::new(dec_idx));
    let ha = highs_arc.clone(); let la = lows_arc.clone(); let ca = closes_arc.clone();
    engine.register_fn("atr_impl", move |period: i64| -> f64 {
        let i = *ci.lock().unwrap();
        let period = (period.max(1)) as usize;
        if i == 0 { return 0.0; }
        let start = i.saturating_sub(period * 3);
        let tr_vals: Vec<f64> = ((start + 1)..=i).map(|j| {
            (ha[j] - la[j]).max((ha[j] - ca[j-1]).abs()).max((la[j] - ca[j-1]).abs())
        }).collect();
        if tr_vals.is_empty() { return 0.0; }
        if tr_vals.len() < period { return tr_vals.iter().sum::<f64>() / tr_vals.len() as f64; }
        let mut atr = tr_vals[..period].iter().sum::<f64>() / period as f64;
        for j in period..tr_vals.len() { atr = (atr * (period - 1) as f64 + tr_vals[j]) / period as f64; }
        atr
    });

    engine.register_fn("set_stop_loss_impl",   |_: f64| {});
    engine.register_fn("set_take_profit_impl", |_: f64| {});
    engine.register_fn("log_impl", |_msg: rhai::Dynamic| {});

    let sb = state.clone();
    engine.register_fn("buy_impl", move |_frac: f64| {
        sb.lock().unwrap().pending_buy = true;
    });
    let ss = state.clone();
    engine.register_fn("sell_impl", move |_frac: f64| {
        ss.lock().unwrap().pending_sell = true;
    });
    // Seed the kv with only non-debug state from the previous window.
    // If the script early-returns, stale debug_* values must not leak
    // into the current window’s indicator output.
    let kv_seed_clean: std::collections::HashMap<String, f64> = kv_seed
        .iter()
        .filter(|(k, _)| !k.starts_with("debug_"))
        .map(|(k, v)| (k.clone(), *v))
        .collect();
    let kv = Arc::new(Mutex::new(kv_seed_clean));
    let kv_set = kv.clone();
    engine.register_fn("set_impl", move |key: String, val: f64| {
        kv_set.lock().unwrap().insert(key, val);
    });
    let kv_get = kv.clone();
    engine.register_fn("get_impl", move |key: String, def: f64| -> f64 {
        kv_get.lock().unwrap().get(&key).copied().unwrap_or(def)
    });

    let ast = engine.compile(&patched_script)
        .map_err(|e| anyhow::anyhow!("Script compile error (patched): {e}"))?;

    // Use the same momentum-derived token price model as the backtest so that
    // live decisions match backtest decisions. The real CLOB price is only used
    // for order execution sizing, not for strategy logic.
    let delta_abs = if window_open > 0.0 {
        ((dec.close - window_open) / window_open * 100.0).abs()
    } else { 0.0 };
    let momentum_token_price = polymarket_token_price(delta_abs);

    let mut ctx_map = rhai::Map::new();
    ctx_map.insert("close".into(),          rhai::Dynamic::from(dec.close));
    ctx_map.insert("open".into(),            rhai::Dynamic::from(dec.open));
    ctx_map.insert("high".into(),            rhai::Dynamic::from(dec.high));
    ctx_map.insert("low".into(),             rhai::Dynamic::from(dec.low));
    ctx_map.insert("volume".into(),          rhai::Dynamic::from(dec.volume));
    ctx_map.insert("index".into(),           rhai::Dynamic::from(dec_idx as i64));
    ctx_map.insert("position".into(),        rhai::Dynamic::from(0.0_f64));
    ctx_map.insert("entry_price".into(),     rhai::Dynamic::from(0.0_f64));
    ctx_map.insert("entry_index".into(),     rhai::Dynamic::from(0i64));
    ctx_map.insert("balance".into(),         rhai::Dynamic::from(1000.0_f64));
    ctx_map.insert("open_positions".into(),  rhai::Dynamic::from(0i64));
    ctx_map.insert("window_open".into(),      rhai::Dynamic::from(window_open));
    ctx_map.insert("window_minutes".into(),   rhai::Dynamic::from(window_minutes as i64));
    ctx_map.insert("token_price".into(),      rhai::Dynamic::from(momentum_token_price));
    ctx_map.insert("threshold".into(),        rhai::Dynamic::from(0.0_f64));
    ctx_map.insert("resolution_value".into(), rhai::Dynamic::from(dec.close));
    ctx_map.insert("value".into(),            rhai::Dynamic::from(dec.close));

    let mut scope = Scope::new();
    engine.call_fn::<()>(&mut scope, &ast, "on_candle", (rhai::Dynamic::from_map(ctx_map),))
        .map_err(|e| anyhow::anyhow!("Script runtime error: {e}"))?;

    let s = state.lock().unwrap();
    let signal = if s.pending_buy {
        "yes".to_string()
    } else if s.pending_sell {
        "no".to_string()
    } else {
        "flat".to_string()
    };

    let kv_final = kv.lock().unwrap().clone();
    let debug = kv_final.clone();
    Ok(LiveSignalResult {
        signal,
        size: 0.25,
        debug,
        kv_state: kv_final,
    })
}

/// Replay the BACKTEST engine on a buffer of 1m candles up to the current
/// decision candle, returning the script's signal and debug values *as the
/// backtester would compute them* at this same decision point.
///
/// Differs from `run_polymarket_live_signal` in three ways:
///  1. State (`ctx.set/get`) accumulates across all completed windows in the
///     buffer (matches BT engine), instead of being reset every call.
///  2. ctx fields (`threshold`, `token_price`, `resolution_value`, `balance`)
///     follow BT semantics (configured threshold, 0.50 for threshold markets,
///     etc.) instead of the live-mode placeholders.
///  3. Completed windows use the FUTURE resolution candle as `ctx.resolution_value`
///     (this is the BT lookahead). The current (incomplete) window falls back
///     to `dec.close` as `resolution_value` so the comparison is visible.
///
/// Use this at decision time to log "BT debug" alongside "LIVE debug" so the
/// operator can spot script divergences between backtest and live execution.
pub fn run_polymarket_bt_signal_preview(
    script_content: &str,
    candles: Vec<Candle>,
    window_minutes: usize,
    decision_minute: Option<i64>,
    resolution_logic: &str,
    threshold: Option<f64>,
    initial_balance: f64,
) -> anyhow::Result<LiveSignalResult> {
    use rhai::{Engine, Scope};
    use std::sync::{Arc, Mutex};

    if candles.is_empty() {
        return Err(anyhow::anyhow!("No candles for BT preview"));
    }

    let check = Engine::new();
    let ast_check = check.compile(script_content)
        .map_err(|e| anyhow::anyhow!("Script compile error: {e}"))?;
    if !ast_check.iter_functions().any(|f| f.name == "on_candle") {
        return Err(anyhow::anyhow!("Binary strategy requires an on_candle(ctx) function."));
    }

    let window_secs = (window_minutes as i64) * 60;
    let last_candle_ts = candles.last().unwrap().open_time_ms / 1000;
    let last_close_ts = last_candle_ts + 60;
    let current_window = last_close_ts - (last_close_ts % window_secs);
    let dec_min = decision_minute.unwrap_or((window_minutes as i64) - 2);

    let ts_to_idx: std::collections::HashMap<i64, usize> = candles.iter().enumerate()
        .map(|(idx, c)| (c.open_time_ms / 1000, idx))
        .collect();

    let closes:  Vec<f64> = candles.iter().map(|c| c.close).collect();
    let highs:   Vec<f64> = candles.iter().map(|c| c.high).collect();
    let lows:    Vec<f64> = candles.iter().map(|c| c.low).collect();
    let volumes: Vec<f64> = candles.iter().map(|c| c.volume).collect();
    let closes_arc  = Arc::new(closes);
    let highs_arc   = Arc::new(highs);
    let lows_arc    = Arc::new(lows);
    let volumes_arc = Arc::new(volumes);

    let patched_script = script_content
        .replace("ctx.rsi(",             "rsi_impl(")
        .replace("ctx.ema(",             "ema_impl(")
        .replace("ctx.atr(",             "atr_impl(")
        .replace("ctx.sma(",             "sma_impl(")
        .replace("ctx.macd_hist(",       "macd_hist_impl(")
        .replace("ctx.close_at(",        "close_at_impl(")
        .replace("ctx.high_at(",         "high_at_impl(")
        .replace("ctx.low_at(",          "low_at_impl(")
        .replace("ctx.volume_at(",       "volume_at_impl(")
        .replace("ctx.set_stop_loss(",   "set_stop_loss_impl(")
        .replace("ctx.set_take_profit(", "set_take_profit_impl(")
        .replace("ctx.buy(",             "buy_impl(")
        .replace("ctx.sell(",            "sell_impl(")
        .replace("ctx.short(",           "sell_impl(")
        .replace("ctx.set(",             "set_impl(")
        .replace("ctx.get(",             "get_impl(")
        .replace("ctx.log(",             "log_impl(");

    #[derive(Clone)]
    struct PreviewState {
        balance:      f64,
        kv:           std::collections::HashMap<String, f64>,
        pending_buy:  bool,
        pending_sell: bool,
    }
    let state = Arc::new(Mutex::new(PreviewState {
        balance: initial_balance,
        kv: std::collections::HashMap::new(),
        pending_buy: false,
        pending_sell: false,
    }));

    let cur_idx = Arc::new(Mutex::new(0usize));

    let mut engine = Engine::new();
    engine.set_max_operations(500_000);
    engine.set_max_call_levels(64);

    let cl = closes_arc.clone();
    engine.register_fn("close_at_impl",  move |idx: i64| -> f64 { cl.get(idx as usize).copied().unwrap_or(0.0) });
    let vl = volumes_arc.clone();
    engine.register_fn("volume_at_impl", move |idx: i64| -> f64 { vl.get(idx as usize).copied().unwrap_or(0.0) });
    let hl = highs_arc.clone();
    engine.register_fn("high_at_impl",   move |idx: i64| -> f64 { hl.get(idx as usize).copied().unwrap_or(0.0) });
    let ll = lows_arc.clone();
    engine.register_fn("low_at_impl",    move |idx: i64| -> f64 { ll.get(idx as usize).copied().unwrap_or(0.0) });

    let ci = cur_idx.clone(); let cr = closes_arc.clone();
    engine.register_fn("rsi_impl", move |period: i64| -> f64 {
        let i = *ci.lock().unwrap();
        let period = period as usize;
        if i < period { return 50.0; }
        let mut gain = 0.0_f64; let mut loss = 0.0_f64;
        for j in (i - period + 1)..=i {
            if j == 0 { continue; }
            let d = cr[j] - cr[j - 1];
            if d > 0.0 { gain += d; } else { loss += d.abs(); }
        }
        gain /= period as f64; loss /= period as f64;
        if loss == 0.0 { 100.0 } else { 100.0 - 100.0 / (1.0 + gain / loss) }
    });

    let ci = cur_idx.clone(); let ce = closes_arc.clone();
    engine.register_fn("ema_impl", move |period: i64| -> f64 {
        let i = *ci.lock().unwrap();
        let period = period as usize;
        if period == 0 { return ce.get(i).copied().unwrap_or(0.0); }
        let k = 2.0 / (period as f64 + 1.0);
        let start = i.saturating_sub(period * 5);
        let mut e = ce[start];
        for j in (start + 1)..=i { e = ce[j] * k + e * (1.0 - k); }
        e
    });

    let ci = cur_idx.clone(); let cs = closes_arc.clone();
    engine.register_fn("sma_impl", move |period: i64| -> f64 {
        let i = *ci.lock().unwrap();
        let period = period as usize;
        if period == 0 { return 0.0; }
        let start = if i + 1 >= period { i + 1 - period } else { 0 };
        cs[start..=i].iter().sum::<f64>() / (i - start + 1) as f64
    });

    let ci_macd = cur_idx.clone(); let ce_macd = closes_arc.clone();
    engine.register_fn("macd_hist_impl", move |fast: i64, slow: i64, signal: i64| -> f64 {
        let i = *ci_macd.lock().unwrap();
        macd_hist_value(&ce_macd, i, fast as usize, slow as usize, signal as usize)
    });

    let ci = cur_idx.clone();
    let ha = highs_arc.clone(); let la = lows_arc.clone(); let ca = closes_arc.clone();
    engine.register_fn("atr_impl", move |period: i64| -> f64 {
        let i = *ci.lock().unwrap();
        let period = (period.max(1)) as usize;
        if i == 0 { return 0.0; }
        let start = i.saturating_sub(period * 3);
        let tr_vals: Vec<f64> = ((start + 1)..=i).map(|j| {
            (ha[j] - la[j]).max((ha[j] - ca[j-1]).abs()).max((la[j] - ca[j-1]).abs())
        }).collect();
        if tr_vals.is_empty() { return 0.0; }
        if tr_vals.len() < period { return tr_vals.iter().sum::<f64>() / tr_vals.len() as f64; }
        let mut atr = tr_vals[..period].iter().sum::<f64>() / period as f64;
        for j in period..tr_vals.len() { atr = (atr * (period - 1) as f64 + tr_vals[j]) / period as f64; }
        atr
    });

    engine.register_fn("set_stop_loss_impl",   |_: f64| {});
    engine.register_fn("set_take_profit_impl", |_: f64| {});
    engine.register_fn("log_impl", |_msg: rhai::Dynamic| {});

    let sb = state.clone();
    engine.register_fn("buy_impl", move |_frac: f64| {
        let mut s = sb.lock().unwrap();
        if !s.pending_buy && !s.pending_sell { s.pending_buy = true; }
    });
    let ss = state.clone();
    engine.register_fn("sell_impl", move |_frac: f64| {
        let mut s = ss.lock().unwrap();
        if !s.pending_buy && !s.pending_sell { s.pending_sell = true; }
    });
    let sset = state.clone();
    engine.register_fn("set_impl", move |key: String, val: f64| {
        sset.lock().unwrap().kv.insert(key, val);
    });
    let sget = state.clone();
    engine.register_fn("get_impl", move |key: String, def: f64| -> f64 {
        sget.lock().unwrap().kv.get(&key).copied().unwrap_or(def)
    });

    let ast = engine.compile(&patched_script)
        .map_err(|e| anyhow::anyhow!("Script compile error (patched): {e}"))?;

    let first_ts = candles[0].open_time_ms / 1000;
    let first_window = align_up_to(first_ts, window_secs);

    // Replay completed windows (window strictly before current_window) to build kv state.
    let mut window_ts = first_window;
    while window_ts < current_window {
        let minute0_ts    = window_ts;
        let decision_ts   = window_ts + dec_min * 60;
        let resolution_ts = window_ts + ((window_minutes as i64) - 1) * 60;
        window_ts += window_secs;

        let (Some(&m0_idx), Some(&dec_idx), Some(&res_idx)) = (
            ts_to_idx.get(&minute0_ts),
            ts_to_idx.get(&decision_ts),
            ts_to_idx.get(&resolution_ts),
        ) else { continue; };

        let window_open = candles[m0_idx].open;
        let dec         = &candles[dec_idx];
        let res_close   = candles[res_idx].close;
        let resolution_value = res_close;
        let thr_val = threshold.unwrap_or(0.0);
        let yes_token_price = match resolution_logic {
            "threshold_above" | "threshold_below" => 0.50_f64,
            _ => {
                let delta_abs = if window_open > 0.0 {
                    ((dec.close - window_open) / window_open * 100.0).abs()
                } else { 0.0 };
                polymarket_token_price(delta_abs)
            }
        };

        {
            let mut s = state.lock().unwrap();
            s.pending_buy = false; s.pending_sell = false;
        }
        *cur_idx.lock().unwrap() = dec_idx;

        let cur_balance = state.lock().unwrap().balance;
        let mut ctx_map = rhai::Map::new();
        ctx_map.insert("close".into(),            rhai::Dynamic::from(dec.close));
        ctx_map.insert("open".into(),             rhai::Dynamic::from(dec.open));
        ctx_map.insert("high".into(),             rhai::Dynamic::from(dec.high));
        ctx_map.insert("low".into(),              rhai::Dynamic::from(dec.low));
        ctx_map.insert("volume".into(),           rhai::Dynamic::from(dec.volume));
        ctx_map.insert("index".into(),            rhai::Dynamic::from(dec_idx as i64));
        ctx_map.insert("position".into(),         rhai::Dynamic::from(0.0_f64));
        ctx_map.insert("entry_price".into(),      rhai::Dynamic::from(0.0_f64));
        ctx_map.insert("entry_index".into(),      rhai::Dynamic::from(0i64));
        ctx_map.insert("balance".into(),          rhai::Dynamic::from(cur_balance));
        ctx_map.insert("open_positions".into(),   rhai::Dynamic::from(0i64));
        ctx_map.insert("window_open".into(),      rhai::Dynamic::from(window_open));
        ctx_map.insert("window_minutes".into(),   rhai::Dynamic::from(window_minutes as i64));
        ctx_map.insert("token_price".into(),      rhai::Dynamic::from(yes_token_price));
        ctx_map.insert("threshold".into(),        rhai::Dynamic::from(thr_val));
        ctx_map.insert("resolution_value".into(), rhai::Dynamic::from(resolution_value));
        ctx_map.insert("value".into(),            rhai::Dynamic::from(dec.close));

        let mut scope = Scope::new();
        let _ = engine.call_fn::<()>(&mut scope, &ast, "on_candle", (rhai::Dynamic::from_map(ctx_map),));
    }

    // Now process the CURRENT window's decision candle (no resolution candle yet).
    let cur_dec_ts = current_window + dec_min * 60;
    let cur_m0_idx = ts_to_idx.get(&current_window).copied();
    let cur_dec_idx = ts_to_idx.get(&cur_dec_ts).copied().or_else(|| {
        candles.iter().enumerate()
            .filter(|(_, c)| (c.open_time_ms / 1000) <= cur_dec_ts)
            .map(|(i, _)| i)
            .last()
    });

    let Some(dec_idx) = cur_dec_idx else {
        let s = state.lock().unwrap();
        return Ok(LiveSignalResult {
            signal: "flat".to_string(),
            size: 0.25,
            debug: s.kv.clone(),
            kv_state: s.kv.clone(),
        });
    };

    let dec = &candles[dec_idx];
    let window_open = cur_m0_idx
        .map(|i| candles[i].open)
        .unwrap_or(dec.open);
    let thr_val = threshold.unwrap_or(0.0);
    let yes_token_price = match resolution_logic {
        "threshold_above" | "threshold_below" => 0.50_f64,
        _ => {
            let delta_abs = if window_open > 0.0 {
                ((dec.close - window_open) / window_open * 100.0).abs()
            } else { 0.0 };
            polymarket_token_price(delta_abs)
        }
    };
    // No future candle available — fall back to dec.close so the script can run.
    let resolution_value = dec.close;

    {
        let mut s = state.lock().unwrap();
        s.pending_buy = false; s.pending_sell = false;
    }
    *cur_idx.lock().unwrap() = dec_idx;

    let cur_balance = state.lock().unwrap().balance;
    let mut ctx_map = rhai::Map::new();
    ctx_map.insert("close".into(),            rhai::Dynamic::from(dec.close));
    ctx_map.insert("open".into(),             rhai::Dynamic::from(dec.open));
    ctx_map.insert("high".into(),             rhai::Dynamic::from(dec.high));
    ctx_map.insert("low".into(),              rhai::Dynamic::from(dec.low));
    ctx_map.insert("volume".into(),           rhai::Dynamic::from(dec.volume));
    ctx_map.insert("index".into(),            rhai::Dynamic::from(dec_idx as i64));
    ctx_map.insert("position".into(),         rhai::Dynamic::from(0.0_f64));
    ctx_map.insert("entry_price".into(),      rhai::Dynamic::from(0.0_f64));
    ctx_map.insert("entry_index".into(),      rhai::Dynamic::from(0i64));
    ctx_map.insert("balance".into(),          rhai::Dynamic::from(cur_balance));
    ctx_map.insert("open_positions".into(),   rhai::Dynamic::from(0i64));
    ctx_map.insert("window_open".into(),      rhai::Dynamic::from(window_open));
    ctx_map.insert("window_minutes".into(),   rhai::Dynamic::from(window_minutes as i64));
    ctx_map.insert("token_price".into(),      rhai::Dynamic::from(yes_token_price));
    ctx_map.insert("threshold".into(),        rhai::Dynamic::from(thr_val));
    ctx_map.insert("resolution_value".into(), rhai::Dynamic::from(resolution_value));
    ctx_map.insert("value".into(),            rhai::Dynamic::from(dec.close));

    let mut scope = Scope::new();
    engine.call_fn::<()>(&mut scope, &ast, "on_candle", (rhai::Dynamic::from_map(ctx_map),))
        .map_err(|e| anyhow::anyhow!("Script runtime error: {e}"))?;

    let s = state.lock().unwrap();
    let signal = if s.pending_buy {
        "yes".to_string()
    } else if s.pending_sell {
        "no".to_string()
    } else {
        "flat".to_string()
    };
    Ok(LiveSignalResult {
        signal,
        size: 0.25,
        debug: s.kv.clone(),
        kv_state: s.kv.clone(),
    })
}

/// Run the Rhai polymarket binary strategy on a pre-built buffer of 1m candles.
/// Called from the live strategy runner at each window boundary.
pub fn run_polymarket_binary_on_candle_buffer(
    script_content: &str,
    candles: Vec<Candle>,
    window_minutes: usize,
    initial_balance: f64,
    fee_pct: f64,
    resolution_logic: &str,
    threshold: Option<f64>,
    max_stake_usd: Option<f64>,
) -> BacktestMetrics {
    run_polymarket_slug_backtest(
        script_content.to_string(),
        candles,
        initial_balance,
        fee_pct,
        window_minutes,
        resolution_logic,
        threshold,
        max_stake_usd,
        None,
        "percent",
        1.0,
    )
    .unwrap_or_else(|e| BacktestMetrics {
        total_return_pct: 0.0, sharpe_ratio: 0.0, max_drawdown_pct: 0.0,
        win_rate_pct: 0.0, total_trades: 0, worst_trades: vec![], all_trades: vec![],
        avg_token_price: None, correct_direction_pct: None, break_even_win_rate: None,
        markets_tested: None, flat_debugs: vec![],
        analysis: format!("Polymarket strategy error: {e}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (TempDir, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let scripts = tmp.path().join("scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        // ctx-based strategy that just holds: no trades, but valid Rhai
        std::fs::write(
            scripts.join("test_strat.rhai"),
            r#"fn on_candle(ctx) { /* hold */ }"#,
        )
        .unwrap();

        // Pre-populate candle cache so tests don't hit the network
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        // Generate 50 synthetic 1-min candles starting 2024-01-01 00:00 UTC
        let base_ms: i64 = 1704067200_000; // 2024-01-01T00:00:00Z
        let candles: Vec<serde_json::Value> = (0..50i64)
            .map(|i| {
                serde_json::json!({
                    "open_time_ms": base_ms + i * 60_000,
                    "open":  42000.0 + (i as f64) * 10.0,
                    "high":  42100.0 + (i as f64) * 10.0,
                    "low":   41900.0 + (i as f64) * 10.0,
                    "close": 42050.0 + (i as f64) * 10.0,
                    "volume": 1.0
                })
            })
            .collect();
        let cache_json = serde_json::to_string(&candles).unwrap();
        std::fs::write(
            data_dir.join("BTCUSDT_1m_2024-01-01_2024-12-31.json"),
            &cache_json,
        )
        .unwrap();

        let ws = tmp.path().to_path_buf();
        (tmp, ws)
    }

    #[tokio::test]
    async fn list_finds_script() {
        let (_tmp, ws) = setup();
        let tool = BacktestListScriptsTool::new(ws);
        let result = tool.execute(json!({})).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("test_strat.rhai"));
    }

    #[tokio::test]
    async fn list_empty_workspace() {
        let tmp = TempDir::new().unwrap();
        let tool = BacktestListScriptsTool::new(tmp.path().to_path_buf());
        let result = tool.execute(json!({})).await.unwrap();
        assert!(result.success);
        // ensure_default_scripts writes bundled scripts on first run, so even
        // an empty workspace will have scripts after the first list call.
        assert!(result.output.contains(".rhai"));
    }

    #[tokio::test]
    async fn run_missing_script() {
        let (_tmp, ws) = setup();
        let tool = BacktestRunTool::new(ws);
        let result = tool
            .execute(json!({ "script": "nonexistent.rhai" }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap_or("").contains("not found"));
    }

    #[tokio::test]
    async fn run_existing_script() {
        let (_tmp, ws) = setup();
        let tool = BacktestRunTool::new(ws);
        let result = tool
            .execute(json!({
                "script": "test_strat.rhai",
                "symbol": "BTCUSDT",
                "from_date": "2024-01-01",
                "to_date": "2024-12-31"
            }))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("Sharpe Ratio"));
        assert!(result.output.contains("Win Rate"));
    }

    #[tokio::test]
    async fn run_missing_param() {
        let (_tmp, ws) = setup();
        let tool = BacktestRunTool::new(ws);
        let result = tool.execute(json!({})).await;
        assert!(result.is_err());
    }
}

use super::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::path::PathBuf;

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
            initial_balance, fee_pct, &self.workspace_dir
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

pub struct BacktestMetrics {
    pub total_return_pct: f64,
    pub sharpe_ratio: f64,
    pub max_drawdown_pct: f64,
    pub win_rate_pct: f64,
    pub total_trades: u32,
    pub worst_trades: Vec<WorstTrade>,
    pub analysis: String,
}

pub struct WorstTrade {
    pub timestamp: String,
    pub side: String,
    pub price: f64,
    pub pnl: f64,
}

// ── Candle from Binance ──────────────────────────────────────────────

#[derive(Clone)]
struct Candle {
    open_time_ms: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
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

    // Fetch from Polymarket CLOB API
    let url = format!(
        "https://clob.polymarket.com/prices-history?market={}&interval={}&fidelity={}&startTs={}&endTs={}",
        condition_id,
        fidelity * 60, // API expects interval in seconds
        fidelity,
        from_ts,
        to_ts
    );

    tracing::info!("[BACKTEST] Fetching Polymarket prices from: {}", url);

    let client = reqwest::Client::new();
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

    // Parse Polymarket response
    // Format: { "history": [ { "t": timestamp, "p": price }, ... ] }
    let candles = parse_polymarket_prices(&body)?;

    tracing::info!("[BACKTEST] Fetched {} Polymarket price points", candles.len());

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

#[derive(Clone)]
struct Trade {
    timestamp: String,
    side: String,
    price: f64,
    size: f64, // base token amount
    pnl: f64,
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

        if has_on_candle {
            // ── ctx-based API ────────────────────────────────────────
            let (cur_balance, cur_position, cur_entry_price, cur_entry_index) = {
                let s = state.lock().unwrap();
                (s.balance, s.position, s.entry_price, s.entry_index)
            };

            // Build ctx map
            let mut ctx = Map::new();
            ctx.insert("close".into(),       Dynamic::from(c.close));
            ctx.insert("open".into(),        Dynamic::from(c.open));
            ctx.insert("high".into(),        Dynamic::from(c.high));
            ctx.insert("low".into(),         Dynamic::from(c.low));
            ctx.insert("volume".into(),      Dynamic::from(c.volume));
            ctx.insert("index".into(),       Dynamic::from(i as i64));
            ctx.insert("position".into(),    Dynamic::from(cur_position));
            ctx.insert("entry_price".into(), Dynamic::from(cur_entry_price));
            ctx.insert("entry_index".into(), Dynamic::from(cur_entry_index));
            ctx.insert("balance".into(),     Dynamic::from(cur_balance));

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
            let atr14_fn   = atr14.to_vec();
            let state_buy  = state.clone();
            let state_sell = state.clone();
            let state_set  = state.clone();
            let state_get  = state.clone();
            let cur_i      = i;

            // ctx.close_at(idx) → f64
            eng2.register_fn("close_at_impl", move |idx: i64| -> f64 {
                closes_fn.get(idx as usize).copied().unwrap_or(0.0)
            });

            let volumes_fn2 = volumes_arc.clone();
            eng2.register_fn("volume_at_impl", move |idx: i64| -> f64 {
                volumes_fn2.get(idx as usize).copied().unwrap_or(0.0)
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

            // atr(period) — returns pre-computed ATR14 (period ignored for now)
            let atr14_fn2 = atr14_fn.clone();
            eng2.register_fn("atr_impl", move |_period: i64| -> f64 {
                atr14_fn2.get(cur_i).copied().unwrap_or(0.0)
            });

            // ctx.buy(size) — size 1.0 = full balance
            let sb = state_buy.clone();
            let buy_price = c.close;
            let _buy_ts   = ts.clone();
            let buy_fee   = fee_pct;
            eng2.register_fn("buy_impl", move |_size: f64| {
                let mut s = sb.lock().unwrap();
                if s.position == 0.0 && s.balance > 0.0 {
                    let fee_factor = 1.0 - buy_fee / 100.0;
                    let qty = (s.balance * fee_factor) / buy_price;
                    s.position    = qty;
                    s.balance     = 0.0;
                    s.entry_price = buy_price;
                    s.entry_index = cur_i as i64;
                }
            });

            // ctx.sell(size) — size 1.0 = close full position
            let ss = state_sell.clone();
            let sell_price = c.close;
            let sell_ts    = ts.clone();
            let sell_fee   = fee_pct;
            eng2.register_fn("sell_impl", move |_size: f64| {
                let mut s = ss.lock().unwrap();
                if s.position > 0.0 {
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
                    });
                    s.balance     = proceeds;
                    s.position    = 0.0;
                    s.entry_price = 0.0;
                } else if s.position < 0.0 {
                    // Close short
                    let fee_factor = 1.0 - sell_fee / 100.0;
                    let pos_abs = s.position.abs();
                    let pnl = (s.entry_price - sell_price) * pos_abs * fee_factor;
                    s.trades.push(Trade {
                        timestamp: sell_ts.clone(),
                        side: "buy_cover".into(),
                        price: sell_price,
                        size: pos_abs,
                        pnl,
                    });
                    s.balance    += pnl;
                    s.position    = 0.0;
                    s.entry_price = 0.0;
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
                .replace("ctx.rsi(",       "rsi_impl(")
                .replace("ctx.ema(",       "ema_impl(")
                .replace("ctx.atr(",       "atr_impl(")
                .replace("ctx.close_at(",  "close_at_impl(")
                .replace("ctx.volume_at(", "volume_at_impl(")
                .replace("ctx.buy(",       "buy_impl(")
                .replace("ctx.sell(",      "sell_impl(")
                .replace("ctx.set(",       "set_impl(")
                .replace("ctx.get(",       "get_impl(");

            let full_script = format!(r#"
{patched_script}

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
on_candle(ctx);
"#,
                patched_script = patched_script,
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

    let analysis = build_analysis(total_return_pct, sharpe_ratio, max_dd, win_rate_pct, total_trades);

    Ok(BacktestMetrics {
        total_return_pct,
        sharpe_ratio,
        max_drawdown_pct: max_dd,
        win_rate_pct,
        total_trades,
        worst_trades,
        analysis,
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
                win_rate_pct: 0.0, total_trades: 0, worst_trades: vec![],
                analysis: format!("Error reading script: {e}"),
            };
        }
    };

    // Fetch historical candles based on market type
    let data_source = if market_type == "polymarket" { "Polymarket" } else { "Binance" };
    tracing::info!("[BACKTEST] Fetching {interval} candles from {data_source} for {symbol}...");

    let candles = if market_type == "polymarket" {
        match fetch_polymarket_candles(symbol, interval, from_date, to_date, workspace_dir).await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("[BACKTEST] Polymarket fetch failed: {e}");
                return BacktestMetrics {
                    total_return_pct: 0.0, sharpe_ratio: 0.0, max_drawdown_pct: 0.0,
                    win_rate_pct: 0.0, total_trades: 0, worst_trades: vec![],
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
                    win_rate_pct: 0.0, total_trades: 0, worst_trades: vec![],
                    analysis: format!(
                        "No candle data returned from Binance for {symbol} ({from_date}→{to_date}). \
                        Check the symbol name and date range."
                    ),
                };
            }
            Err(e) => {
                tracing::error!("[BACKTEST] Binance fetch failed: {e}");
                return BacktestMetrics {
                    total_return_pct: 0.0, sharpe_ratio: 0.0, max_drawdown_pct: 0.0,
                    win_rate_pct: 0.0, total_trades: 0, worst_trades: vec![],
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

    // Run Rhai engine in blocking thread (CPU-bound)
    tracing::info!("[BACKTEST] Running Rhai engine on {} candles...", num_candles);
    match tokio::task::spawn_blocking(move || {
        run_rhai_backtest(script_content, candles, initial_balance, fee_pct)
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
                win_rate_pct: 0.0, total_trades: 0, worst_trades: vec![],
                analysis: format!("Rhai execution error: {e}"),
            }
        }
        Err(e) => {
            tracing::error!("[BACKTEST] Backtest task panicked: {e}");
            BacktestMetrics {
                total_return_pct: 0.0, sharpe_ratio: 0.0, max_drawdown_pct: 0.0,
                win_rate_pct: 0.0, total_trades: 0, worst_trades: vec![],
                analysis: format!("Backtest task panicked: {e}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (TempDir, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let scripts = tmp.path().join("scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        std::fs::write(
            scripts.join("test_strat.rhai"),
            "// Buy low sell high\nlet rsi = get_rsi(\"BTCUSDT\", 14);",
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
        assert!(result.output.contains("No .rhai scripts"));
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

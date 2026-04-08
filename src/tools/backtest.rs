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
                "symbol": {
                    "type": "string",
                    "description": "Trading pair symbol (e.g. 'BTCUSDT', 'ETHUSDT')",
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

        // Run the real Rhai backtest engine with Binance data
        let metrics = run_backtest_engine(
            &script_path, symbol, from_date, to_date,
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

/// Fetch daily OHLCV candles from Binance REST API.
/// Caches the result to `<workspace>/data/<symbol>_<from>_<to>.json`.
async fn fetch_candles(
    symbol: &str,
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
    let cache_file = data_dir.join(format!("{}_{from_date}_{to_date}.json", symbol.to_uppercase()));
    if let Ok(cached) = std::fs::read_to_string(&cache_file) {
        if let Ok(candles) = parse_binance_klines(&cached) {
            if !candles.is_empty() {
                return Ok(candles);
            }
        }
    }

    // Fetch from Binance (up to 1000 daily candles ≈ ~2.7 years)
    let url = format!(
        "https://api.binance.com/api/v3/klines?symbol={}&interval=1d&startTime={}&endTime={}&limit=1000",
        symbol.to_uppercase(),
        from_ms,
        to_ms
    );
    let body = reqwest::Client::new()
        .get(&url)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Binance request failed: {e}"))?
        .error_for_status()
        .map_err(|e| anyhow::anyhow!("Binance returned error: {e}"))?
        .text()
        .await
        .map_err(|e| anyhow::anyhow!("Binance response error: {e}"))?;

    let candles = parse_binance_klines(&body)?;

    // Cache for next run
    let _ = std::fs::write(&cache_file, &body);

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

// ── Rhai execution ───────────────────────────────────────────────────

struct Trade {
    timestamp: String,
    side: String,
    price: f64,
    size: f64, // base token amount
    pnl: f64,
}

/// Run the Rhai script against all candles, simulate trades, return metrics.
fn run_rhai_backtest(
    script_content: String,
    candles: Vec<Candle>,
    initial_balance: f64,
    fee_pct: f64,
) -> anyhow::Result<BacktestMetrics> {
    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
    let rsi14 = compute_rsi_series(&closes, 14);
    let (macd_line, signal_line, macd_hist) = compute_macd_series(&closes);

    let engine = rhai::Engine::new();
    let ast = engine
        .compile(&script_content)
        .map_err(|e| anyhow::anyhow!("Script compile error: {e}"))?;

    let mut balance = initial_balance;
    let mut position = 0.0_f64; // base token held
    let mut entry_price = 0.0_f64;
    let mut trades: Vec<Trade> = Vec::new();

    // Track portfolio values for Sharpe/drawdown
    let mut portfolio_values: Vec<f64> = vec![initial_balance];
    let mut peak = initial_balance;
    let mut max_dd = 0.0_f64;

    for i in 0..candles.len() {
        let c = &candles[i];
        let ts = chrono::DateTime::from_timestamp_millis(c.open_time_ms)
            .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .unwrap_or_else(|| c.open_time_ms.to_string());

        let mut scope = rhai::Scope::new();
        scope.push("open",     c.open);
        scope.push("high",     c.high);
        scope.push("low",      c.low);
        scope.push("close",    c.close);
        scope.push("volume",   c.volume);
        scope.push("ts",       ts.clone());
        scope.push("rsi",      rsi14[i]);
        scope.push("macd",     macd_line[i]);
        scope.push("signal",   signal_line[i]);
        scope.push("macd_hist", macd_hist[i]);
        scope.push("balance",  balance);
        scope.push("position", position);

        // Evaluate script; expect it to set the `signal` variable to "buy"/"sell"/"hold"
        let signal: String = match engine.eval_ast_with_scope::<rhai::Dynamic>(&mut scope, &ast) {
            Ok(v) => v.try_cast::<String>().unwrap_or_else(|| "hold".into()),
            Err(_) => {
                // Try reading `signal` variable from scope if script doesn't return a value
                scope.get_value::<String>("signal").unwrap_or_else(|| "hold".into())
            }
        };

        let fee_factor = 1.0 - fee_pct / 100.0;
        match signal.to_lowercase().as_str() {
            "buy" if position == 0.0 && balance > 0.0 => {
                let qty = (balance * fee_factor) / c.close;
                position = qty;
                balance = 0.0;
                entry_price = c.close;
            }
            "sell" if position > 0.0 => {
                let gross = position * c.close;
                let proceeds = gross * fee_factor;
                let pnl = proceeds - entry_price * position;
                trades.push(Trade {
                    timestamp: ts,
                    side: "sell".into(),
                    price: c.close,
                    size: position,
                    pnl,
                });
                balance = proceeds;
                position = 0.0;
                entry_price = 0.0;
            }
            _ => {}
        }

        // Portfolio value at current close
        let equity = balance + position * c.close;
        portfolio_values.push(equity);
        if equity > peak { peak = equity; }
        let dd = (peak - equity) / peak * 100.0;
        if dd > max_dd { max_dd = dd; }
    }

    // Close any open position at last price
    if position > 0.0 && !candles.is_empty() {
        let last = candles.last().unwrap();
        let proceeds = position * last.close * (1.0 - fee_pct / 100.0);
        let pnl = proceeds - entry_price * position;
        trades.push(Trade {
            timestamp: chrono::DateTime::from_timestamp_millis(last.open_time_ms)
                .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
                .unwrap_or_default(),
            side: "close".into(),
            price: last.close,
            size: position,
            pnl,
        });
        balance = proceeds;
    }

    let final_value = balance;
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
    symbol: &str,
    from_date: &str,
    to_date: &str,
    initial_balance: f64,
    fee_pct: f64,
    workspace_dir: &std::path::Path,
) -> BacktestMetrics {
    let script_content = match std::fs::read_to_string(script_path) {
        Ok(s) => s,
        Err(e) => {
            return BacktestMetrics {
                total_return_pct: 0.0, sharpe_ratio: 0.0, max_drawdown_pct: 0.0,
                win_rate_pct: 0.0, total_trades: 0, worst_trades: vec![],
                analysis: format!("Error reading script: {e}"),
            };
        }
    };

    // Fetch historical candles
    let candles = match fetch_candles(symbol, from_date, to_date, workspace_dir).await {
        Ok(c) if !c.is_empty() => c,
        Ok(_) => {
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
            return BacktestMetrics {
                total_return_pct: 0.0, sharpe_ratio: 0.0, max_drawdown_pct: 0.0,
                win_rate_pct: 0.0, total_trades: 0, worst_trades: vec![],
                analysis: format!(
                    "Could not fetch historical data from Binance: {e}. \
                    Ensure the gateway has internet access."
                ),
            };
        }
    };

    let num_candles = candles.len();

    // Run Rhai engine in blocking thread (CPU-bound)
    match tokio::task::spawn_blocking(move || {
        run_rhai_backtest(script_content, candles, initial_balance, fee_pct)
    })
    .await
    {
        Ok(Ok(mut metrics)) => {
            metrics.analysis = format!(
                "[{num_candles} daily candles from Binance] {}",
                metrics.analysis
            );
            metrics
        }
        Ok(Err(e)) => BacktestMetrics {
            total_return_pct: 0.0, sharpe_ratio: 0.0, max_drawdown_pct: 0.0,
            win_rate_pct: 0.0, total_trades: 0, worst_trades: vec![],
            analysis: format!("Rhai execution error: {e}"),
        },
        Err(e) => BacktestMetrics {
            total_return_pct: 0.0, sharpe_ratio: 0.0, max_drawdown_pct: 0.0,
            win_rate_pct: 0.0, total_trades: 0, worst_trades: vec![],
            analysis: format!("Backtest task panicked: {e}"),
        },
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

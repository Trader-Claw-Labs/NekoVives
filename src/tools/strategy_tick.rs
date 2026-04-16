use super::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::path::PathBuf;

pub struct StrategyTickTool {
    pub workspace_dir: PathBuf,
}

#[async_trait]
impl Tool for StrategyTickTool {
    fn name(&self) -> &str { "strategy_tick" }

    fn description(&self) -> &str {
        "Run one tick of a Rhai trading strategy against recent historical data and return the \
        current signal (buy/sell/flat) plus performance metrics. \
        Use this to check what a strategy would signal right now without starting a live runner. \
        For crypto, fetches last N days from Binance. For polymarket, from CLOB API."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "script": { "type": "string", "description": "Script filename (e.g. 'crypto_4min.rhai') or full path" },
                "market_type": { "type": "string", "enum": ["crypto", "polymarket"] },
                "symbol": { "type": "string", "description": "Symbol e.g. BTCUSDT or Polymarket condition_id/slug" },
                "interval": { "type": "string", "description": "Candle interval e.g. '4m', '5m', '1h'" },
                "warmup_days": { "type": "integer", "description": "Days of history to use (default 30)", "default": 30 },
                "initial_balance": { "type": "number", "default": 10000 },
                "fee_pct": { "type": "number", "default": 0.1 }
            },
            "required": ["script", "market_type", "symbol", "interval"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let script = args.get("script").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let market_type = args.get("market_type").and_then(|v| v.as_str()).unwrap_or("crypto");
        let symbol = args.get("symbol").and_then(|v| v.as_str()).unwrap_or("BTCUSDT");
        let interval = args.get("interval").and_then(|v| v.as_str()).unwrap_or("1h");
        let warmup_days = args.get("warmup_days").and_then(|v| v.as_i64()).unwrap_or(30);
        let initial_balance = args.get("initial_balance").and_then(|v| v.as_f64()).unwrap_or(10000.0);
        let fee_pct = args.get("fee_pct").and_then(|v| v.as_f64()).unwrap_or(0.1);

        let script_path = {
            let p = std::path::Path::new(&script);
            if p.is_absolute() || p.exists() { p.to_path_buf() }
            else { self.workspace_dir.join("scripts").join(&script) }
        };

        if !script_path.exists() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Script not found: {}", script_path.display())),
            });
        }

        let from = (chrono::Utc::now() - chrono::Duration::days(warmup_days)).format("%Y-%m-%d").to_string();
        let to = chrono::Utc::now().format("%Y-%m-%d").to_string();

        let metrics = crate::tools::backtest::run_backtest_engine(
            &script_path, market_type, symbol, interval, &from, &to,
            initial_balance, fee_pct, &self.workspace_dir,
        ).await;

        let last_signal = metrics.all_trades.last().map(|t| t.side.as_str()).unwrap_or("flat");
        let balance = initial_balance * (1.0 + metrics.total_return_pct / 100.0);

        Ok(ToolResult {
            success: true,
            output: format!(
                "Strategy Tick: {} on {} ({})\n\
                Signal: {}\n\
                Period: {} → {}\n\
                Total Return: {:.2}% | Balance: ${:.2}\n\
                Trades: {} | Win Rate: {:.1}% | Sharpe: {:.2}\n\
                Max Drawdown: {:.2}%\n\
                Analysis: {}",
                script, symbol, interval,
                last_signal.to_uppercase(),
                from, to,
                metrics.total_return_pct, balance,
                metrics.total_trades, metrics.win_rate_pct, metrics.sharpe_ratio,
                metrics.max_drawdown_pct,
                metrics.analysis,
            ),
            error: None,
        })
    }
}

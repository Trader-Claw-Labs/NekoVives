use super::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::path::PathBuf;

pub struct LiveStrategiesListTool {
    workspace_dir: PathBuf,
}

impl LiveStrategiesListTool {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }
}

#[async_trait]
impl Tool for LiveStrategiesListTool {
    fn name(&self) -> &str {
        "live_strategies_list"
    }

    fn description(&self) -> &str {
        "List all live/paper trading strategies and their current status. \
        Shows which strategies are running in live mode (real orders) vs dry-run/paper mode (simulated), \
        their script, symbol, market type, current balance, P&L, win rate, last signal, and status. \
        Use this to answer questions about active strategies, how many are running, or their performance."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "mode_filter": {
                    "type": "string",
                    "description": "Optional filter: 'live' for real-money strategies, 'paper' for dry-run/paper strategies. Omit to list all.",
                    "enum": ["live", "paper"]
                },
                "status_filter": {
                    "type": "string",
                    "description": "Optional filter by runner status: 'running', 'stopped', 'error'. Omit to list all.",
                    "enum": ["running", "stopped", "error", "starting"]
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let file = self.workspace_dir.join("live_strategies.json");

        let data = match tokio::fs::read_to_string(&file).await {
            Ok(d) => d,
            Err(_) => {
                return Ok(ToolResult {
                    success: true,
                    output: "No live strategies found. No strategies have been created yet.".to_string(),
                    error: None,
                });
            }
        };

        let runners: Vec<serde_json::Value> = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Failed to parse live_strategies.json: {e}")),
                });
            }
        };

        let mode_filter = args.get("mode_filter").and_then(|v| v.as_str()).map(str::to_lowercase);
        let status_filter = args.get("status_filter").and_then(|v| v.as_str()).map(str::to_lowercase);

        let mut live_count = 0usize;
        let mut paper_count = 0usize;
        let mut running_count = 0usize;
        let mut summaries = Vec::new();

        let empty = json!({});
        for runner in &runners {
            let config = runner.get("config").unwrap_or(&empty);
            let status = runner.get("status").unwrap_or(&empty);
            let result = runner.get("result");

            let mode = config.get("mode").and_then(|v| v.as_str()).unwrap_or("unknown");
            let runner_status = status.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");

            match mode {
                "live" => live_count += 1,
                "paper" => paper_count += 1,
                _ => {}
            }
            if runner_status == "running" {
                running_count += 1;
            }

            // Apply filters
            if let Some(ref mf) = mode_filter {
                if mode != mf.as_str() {
                    continue;
                }
            }
            if let Some(ref sf) = status_filter {
                if runner_status != sf.as_str() {
                    continue;
                }
            }

            let id = config.get("id").and_then(|v| v.as_str()).unwrap_or("?");
            let name = config.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let script = config.get("script").and_then(|v| v.as_str()).unwrap_or("?");
            let symbol = config.get("symbol").and_then(|v| v.as_str()).unwrap_or("?");
            let interval = config.get("interval").and_then(|v| v.as_str()).unwrap_or("?");
            let market_type = config.get("market_type").and_then(|v| v.as_str()).unwrap_or("?");
            let started_at = status.get("started_at").and_then(|v| v.as_str()).unwrap_or("?");
            let last_tick = status.get("last_tick_at").and_then(|v| v.as_str()).unwrap_or("never");
            let error = status.get("error").and_then(|v| v.as_str());

            let mut entry = format!(
                "- [{runner_status}] {name} (id: {id})\n  Mode: {mode} | Market: {market_type} | Symbol: {symbol} | Interval: {interval}\n  Script: {script}\n  Started: {started_at} | Last tick: {last_tick}"
            );

            if let Some(r) = result {
                let balance = r.get("balance").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let ret_pct = r.get("total_return_pct").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let win_rate = r.get("win_rate_pct").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let trades = r.get("total_trades").and_then(|v| v.as_u64()).unwrap_or(0);
                let last_signal = r.get("last_signal").and_then(|v| v.as_str()).unwrap_or("none");

                let live_total = r.get("live_total_trades").and_then(|v| v.as_u64()).unwrap_or(0);
                let live_wins = r.get("live_wins").and_then(|v| v.as_u64()).unwrap_or(0);
                let live_win_rate = if live_total > 0 {
                    format!("{:.1}%", live_wins as f64 / live_total as f64 * 100.0)
                } else {
                    "n/a".to_string()
                };

                entry.push_str(&format!(
                    "\n  Balance: ${balance:.2} | Return: {ret_pct:+.2}% | Win rate: {win_rate:.1}% | Trades: {trades} | Last signal: {last_signal}"
                ));
                if mode == "live" && live_total > 0 {
                    entry.push_str(&format!(
                        "\n  Live orders: {live_total} | Live win rate: {live_win_rate}"
                    ));
                }
            }

            if let Some(err) = error {
                entry.push_str(&format!("\n  Error: {err}"));
            }

            summaries.push(entry);
        }

        let total = runners.len();
        let header = format!(
            "=== Live Strategies Summary ===\nTotal: {total} | Live (real money): {live_count} | Paper/Dry-run: {paper_count} | Currently running: {running_count}\n"
        );

        let body = if summaries.is_empty() {
            if mode_filter.is_some() || status_filter.is_some() {
                "No strategies match the given filters.".to_string()
            } else {
                "No strategies configured.".to_string()
            }
        } else {
            summaries.join("\n")
        };

        Ok(ToolResult {
            success: true,
            output: format!("{header}\n{body}"),
            error: None,
        })
    }
}

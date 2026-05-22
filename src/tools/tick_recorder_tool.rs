use super::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::path::PathBuf;

/// Tool to start, stop, and query the 1-Hz Polymarket tick recorder.
///
/// The recorder streams YES/NO bid-ask prices from the Polymarket CLOB and
/// the BTC spot price from Binance every second, writing JSONL rows to
/// `<workspace>/data/ticks/<slug>/<YYYY-MM-DD>.jsonl`.
/// Up to 7 days of data are retained for backtesting.
pub struct TickRecorderTool {
    pub workspace_dir: PathBuf,
}

#[async_trait]
impl Tool for TickRecorderTool {
    fn name(&self) -> &str {
        "tick_recorder"
    }

    fn description(&self) -> &str {
        "Start, stop, or query the 1-Hz Polymarket tick recorder. \
        When running, it saves every second: Polymarket YES/NO bid/ask, Binance spot price, \
        and optional Chainlink oracle price to JSONL files for later backtesting. \
        Use action='start' to begin recording, 'stop' to halt, 'status' to list active recorders, \
        and 'read' to get the last N ticks from today's log."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["start", "stop", "status", "read"],
                    "description": "Action to perform"
                },
                "slug": {
                    "type": "string",
                    "description": "Market slug, e.g. 'btc_5m'. Required for start/stop/read."
                },
                "condition_id": {
                    "type": "string",
                    "description": "Polymarket YES-token condition_id (hex). Required for start."
                },
                "binance_symbol": {
                    "type": "string",
                    "description": "Binance symbol, e.g. 'BTCUSDT'. Default: 'BTCUSDT'.",
                    "default": "BTCUSDT"
                },
                "chainlink_url": {
                    "type": "string",
                    "description": "Optional Chainlink REST endpoint URL for oracle comparison."
                },
                "last_n": {
                    "type": "integer",
                    "description": "For action='read': how many recent ticks to return. Default: 60.",
                    "default": 60
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("status");

        match action {
            "start" => {
                let slug = match args.get("slug").and_then(|v| v.as_str()) {
                    Some(s) => s.to_string(),
                    None => {
                        return Ok(ToolResult {
                            success: false,
                            output: String::new(),
                            error: Some("'slug' is required for action='start'".to_string()),
                        });
                    }
                };
                let condition_id = args
                    .get("condition_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&slug)
                    .to_string();
                let binance_symbol = args
                    .get("binance_symbol")
                    .and_then(|v| v.as_str())
                    .unwrap_or("BTCUSDT")
                    .to_string();
                let chainlink_url = args
                    .get("chainlink_url")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let mut cfg = crate::tick_recorder::TickRecorderConfig::new(
                    &slug,
                    &condition_id,
                    &binance_symbol,
                    &self.workspace_dir,
                );
                cfg.chainlink_url = chainlink_url;

                crate::tick_recorder::start_recorder(cfg).await;

                Ok(ToolResult {
                    success: true,
                    output: format!(
                        "Tick recorder started for '{}' (symbol={}).\n\
                        Writing to: {}/data/ticks/{}/\n\
                        Records YES/NO bid-ask, Binance price, and oracle lag every second.\n\
                        Use action='stop' to halt or action='read' to get recent ticks.",
                        slug,
                        binance_symbol,
                        self.workspace_dir.display(),
                        slug
                    ),
                    error: None,
                })
            }

            "stop" => {
                let slug = match args.get("slug").and_then(|v| v.as_str()) {
                    Some(s) => s.to_string(),
                    None => {
                        return Ok(ToolResult {
                            success: false,
                            output: String::new(),
                            error: Some("'slug' is required for action='stop'".to_string()),
                        });
                    }
                };
                let stopped = crate::tick_recorder::stop_recorder(&slug).await;
                Ok(ToolResult {
                    success: true,
                    output: if stopped {
                        format!("Tick recorder for '{}' stopped.", slug)
                    } else {
                        format!("No active tick recorder found for '{}'.", slug)
                    },
                    error: None,
                })
            }

            "status" => {
                let running = crate::tick_recorder::running_recorders().await;
                if running.is_empty() {
                    Ok(ToolResult {
                        success: true,
                        output: "No tick recorders are currently running.".to_string(),
                        error: None,
                    })
                } else {
                    Ok(ToolResult {
                        success: true,
                        output: format!(
                            "Active tick recorders ({}):\n{}",
                            running.len(),
                            running.iter().map(|s| format!("  • {s}")).collect::<Vec<_>>().join("\n")
                        ),
                        error: None,
                    })
                }
            }

            "read" => {
                let slug = match args.get("slug").and_then(|v| v.as_str()) {
                    Some(s) => s.to_string(),
                    None => {
                        return Ok(ToolResult {
                            success: false,
                            output: String::new(),
                            error: Some("'slug' is required for action='read'".to_string()),
                        });
                    }
                };
                let last_n = args.get("last_n").and_then(|v| v.as_u64()).unwrap_or(60) as usize;

                match crate::tick_recorder::read_recent_ticks(&self.workspace_dir, &slug, last_n).await {
                    Ok(ticks) => {
                        if ticks.is_empty() {
                            Ok(ToolResult {
                                success: true,
                                output: format!("No ticks found for '{}' today.", slug),
                                error: None,
                            })
                        } else {
                            let first = ticks.first().unwrap();
                            let last = ticks.last().unwrap();
                            let avg_yes_mid: f64 = ticks.iter().filter(|t| t.yes_mid > 0.0).map(|t| t.yes_mid).sum::<f64>()
                                / ticks.iter().filter(|t| t.yes_mid > 0.0).count().max(1) as f64;
                            let avg_lag: f64 = ticks.iter().map(|t| t.oracle_lag_ms as f64).sum::<f64>()
                                / ticks.len() as f64;
                            let last_binance = last.binance_price;
                            let last_chainlink = last.chainlink_price;
                            let spread_pct = if last.yes_bid > 0.0 && last.yes_ask > 0.0 {
                                (last.yes_ask - last.yes_bid) * 100.0
                            } else { 0.0 };

                            Ok(ToolResult {
                                success: true,
                                output: format!(
                                    "Tick data for '{}' — last {} ticks:\n\
                                    Period: {} → {}\n\
                                    Last YES mid: {:.4}  avg YES mid: {:.4}\n\
                                    Last YES bid/ask: {:.4} / {:.4}  spread: {:.2}¢\n\
                                    Binance price: ${:.2}  Chainlink: ${:.2}\n\
                                    Avg oracle lag: {:.0}ms\n\
                                    Window: {} ({} secs left)",
                                    slug,
                                    ticks.len(),
                                    chrono::DateTime::from_timestamp_millis(first.ts_ms)
                                        .map(|d: chrono::DateTime<chrono::Utc>| d.format("%H:%M:%S").to_string())
                                        .unwrap_or_default(),
                                    chrono::DateTime::from_timestamp_millis(last.ts_ms)
                                        .map(|d: chrono::DateTime<chrono::Utc>| d.format("%H:%M:%S").to_string())
                                        .unwrap_or_default(),
                                    last.yes_mid, avg_yes_mid,
                                    last.yes_bid, last.yes_ask, spread_pct,
                                    last_binance, last_chainlink,
                                    avg_lag,
                                    chrono::DateTime::from_timestamp(last.window_ts, 0)
                                        .map(|d: chrono::DateTime<chrono::Utc>| d.format("%H:%M:%S UTC").to_string())
                                        .unwrap_or_default(),
                                    last.window_secs_left,
                                ),
                                error: None,
                            })
                        }
                    }
                    Err(e) => Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some(format!("Failed to read ticks for '{}': {e}", slug)),
                    }),
                }
            }

            _ => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Unknown action '{action}'. Use: start, stop, status, read")),
            }),
        }
    }
}

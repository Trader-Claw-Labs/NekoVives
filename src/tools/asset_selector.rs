//! Dynamic asset selector — rolling 30-day win-rate tracker.
//!
//! Tracks per-(script × symbol) performance in a JSON file and computes
//! recommended capital allocation weights proportional to each pair's
//! rolling win rate. This allows the operator to concentrate capital on
//! the best-performing asset at any time without manually tuning strategies.
//!
//! Storage: `<workspace>/data/asset_selector_stats.json`
//!
//! Schema:
//! ```json
//! {
//!   "entries": [
//!     {
//!       "script":      "polymarket_all_updown_5m_adaptive.rhai",
//!       "symbol":      "XRPUSDT",
//!       "window_ts":   1748000000,
//!       "won":         true,
//!       "pnl":         12.5
//!     }, ...
//!   ]
//! }
//! ```
//!
//! The tool exposes three actions:
//!   - `record`   — append a resolved trade outcome
//!   - `weights`  — return current allocation weights (0..1) per script×symbol
//!   - `summary`  — human-readable performance table

use super::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;

// ── Data model ────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TradeEntry {
    pub script: String,
    pub symbol: String,
    /// Window start Unix timestamp (seconds), 300-aligned for 5m markets.
    pub window_ts: i64,
    pub won: bool,
    pub pnl: f64,
}

#[derive(Serialize, Deserialize, Default, Debug)]
struct Stats {
    entries: Vec<TradeEntry>,
}

const FILE_NAME: &str = "asset_selector_stats.json";
const ROLLING_DAYS: i64 = 30;

// ── Tool ──────────────────────────────────────────────────────────────────────

pub struct AssetSelectorTool {
    pub workspace_dir: PathBuf,
}

impl AssetSelectorTool {
    fn stats_path(&self) -> PathBuf {
        self.workspace_dir.join("data").join(FILE_NAME)
    }

    async fn load(&self) -> Stats {
        match tokio::fs::read_to_string(self.stats_path()).await {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Stats::default(),
        }
    }

    async fn save(&self, stats: &Stats) -> anyhow::Result<()> {
        let dir = self.workspace_dir.join("data");
        tokio::fs::create_dir_all(&dir).await?;
        let json = serde_json::to_string_pretty(stats)?;
        tokio::fs::write(self.stats_path(), json).await?;
        Ok(())
    }
}

#[async_trait]
impl Tool for AssetSelectorTool {
    fn name(&self) -> &str {
        "asset_selector"
    }

    fn description(&self) -> &str {
        "Dynamic asset selector: track trade outcomes per (script × symbol) and get \
        rolling 30-day win-rate based capital allocation weights. \
        Use action='record' to log a resolved trade, action='weights' to get the \
        recommended allocation weights (higher = better performing asset), and \
        action='summary' to see a performance table."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["record", "weights", "summary", "clear"],
                    "description": "Action to perform"
                },
                "script": {
                    "type": "string",
                    "description": "Strategy script filename, e.g. 'polymarket_all_updown_5m_adaptive.rhai'"
                },
                "symbol": {
                    "type": "string",
                    "description": "Binance symbol, e.g. 'XRPUSDT'"
                },
                "window_ts": {
                    "type": "integer",
                    "description": "Window start Unix timestamp (seconds). Required for action='record'."
                },
                "won": {
                    "type": "boolean",
                    "description": "Whether the trade was a win. Required for action='record'."
                },
                "pnl": {
                    "type": "number",
                    "description": "Trade P&L in USDC. Required for action='record'.",
                    "default": 0.0
                },
                "min_trades": {
                    "type": "integer",
                    "description": "Minimum trades required for a pair to receive non-zero weight. Default: 10.",
                    "default": 10
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("summary");

        match action {
            "record" => {
                let script = match args.get("script").and_then(|v| v.as_str()) {
                    Some(s) => s.to_string(),
                    None => return Ok(err("'script' is required for action='record'")),
                };
                let symbol = match args.get("symbol").and_then(|v| v.as_str()) {
                    Some(s) => s.to_string(),
                    None => return Ok(err("'symbol' is required for action='record'")),
                };
                let window_ts = args.get("window_ts").and_then(|v| v.as_i64())
                    .unwrap_or_else(|| chrono::Utc::now().timestamp());
                let won = args.get("won").and_then(|v| v.as_bool()).unwrap_or(false);
                let pnl = args.get("pnl").and_then(|v| v.as_f64()).unwrap_or(0.0);

                let mut stats = self.load().await;
                // Deduplicate by (script, symbol, window_ts) — idempotent re-record
                stats.entries.retain(|e| {
                    !(e.script == script && e.symbol == symbol && e.window_ts == window_ts)
                });
                stats.entries.push(TradeEntry { script, symbol, window_ts, won, pnl });

                // Prune entries older than 30 days
                let cutoff = chrono::Utc::now().timestamp() - ROLLING_DAYS * 86_400;
                stats.entries.retain(|e| e.window_ts >= cutoff);

                self.save(&stats).await?;
                Ok(ToolResult {
                    success: true,
                    output: format!(
                        "Recorded: {} {} → {} (pnl={:+.2}). Total entries: {}",
                        stats.entries.last().unwrap().script,
                        stats.entries.last().unwrap().symbol,
                        if won { "WIN" } else { "LOSS" },
                        pnl,
                        stats.entries.len()
                    ),
                    error: None,
                })
            }

            "weights" => {
                let min_trades = args.get("min_trades").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
                let stats = self.load().await;
                let weights = compute_weights(&stats, min_trades);

                if weights.is_empty() {
                    return Ok(ToolResult {
                        success: true,
                        output: format!(
                            "No pairs have ≥{min_trades} trades yet. Equal weight applied to all."
                        ),
                        error: None,
                    });
                }

                let mut rows: Vec<String> = weights
                    .iter()
                    .map(|(k, w)| format!("  {:<52} weight={:.3}", k, w))
                    .collect();
                rows.sort();
                Ok(ToolResult {
                    success: true,
                    output: format!(
                        "Allocation weights (rolling {ROLLING_DAYS}d, min {min_trades} trades):\n{}",
                        rows.join("\n")
                    ),
                    error: None,
                })
            }

            "summary" => {
                let stats = self.load().await;
                let cutoff = chrono::Utc::now().timestamp() - ROLLING_DAYS * 86_400;
                let recent: Vec<&TradeEntry> = stats.entries.iter()
                    .filter(|e| e.window_ts >= cutoff)
                    .collect();

                if recent.is_empty() {
                    return Ok(ToolResult {
                        success: true,
                        output: "No trade history found in the last 30 days.".to_string(),
                        error: None,
                    });
                }

                // Group by script × symbol
                let mut groups: std::collections::HashMap<String, (usize, usize, f64)> =
                    std::collections::HashMap::new();
                for e in &recent {
                    let key = format!("{} × {}", e.script, e.symbol);
                    let entry = groups.entry(key).or_insert((0, 0, 0.0));
                    entry.0 += 1;
                    if e.won { entry.1 += 1; }
                    entry.2 += e.pnl;
                }

                let mut rows: Vec<String> = groups
                    .iter()
                    .map(|(k, (total, wins, pnl))| {
                        let wr = if *total > 0 { *wins as f64 / *total as f64 * 100.0 } else { 0.0 };
                        format!("  {:<52} {:>4} trades  {:>5.1}% WR  {:>+8.2} USDC", k, total, wr, pnl)
                    })
                    .collect();
                rows.sort();

                Ok(ToolResult {
                    success: true,
                    output: format!(
                        "Asset selector — last {ROLLING_DAYS} days ({} trades across {} pairs):\n{}\n\
                        \nColumns: script × symbol | trades | win rate | total P&L",
                        recent.len(),
                        groups.len(),
                        rows.join("\n")
                    ),
                    error: None,
                })
            }

            "clear" => {
                let stats = Stats::default();
                self.save(&stats).await?;
                Ok(ToolResult {
                    success: true,
                    output: "Asset selector history cleared.".to_string(),
                    error: None,
                })
            }

            _ => Ok(err(&format!("Unknown action '{action}'. Use: record, weights, summary, clear"))),
        }
    }
}

// ── Weight computation ─────────────────────────────────────────────────────────

/// Returns a map of "script × symbol" → allocation weight (sum-normalized to 1.0).
/// Pairs with fewer than `min_trades` receive weight=0.
pub fn compute_weights(stats: &Stats, min_trades: usize) -> std::collections::HashMap<String, f64> {
    let cutoff = chrono::Utc::now().timestamp() - ROLLING_DAYS * 86_400;
    let recent: Vec<&TradeEntry> = stats.entries.iter()
        .filter(|e| e.window_ts >= cutoff)
        .collect();

    let mut group_wr: std::collections::HashMap<String, (usize, usize)> = std::collections::HashMap::new();
    for e in &recent {
        let key = format!("{} × {}", e.script, e.symbol);
        let entry = group_wr.entry(key).or_insert((0, 0));
        entry.0 += 1;
        if e.won { entry.1 += 1; }
    }

    // Raw scores: win_rate for pairs with enough data, 0 for others
    let mut raw: Vec<(String, f64)> = group_wr.iter()
        .filter_map(|(k, (total, wins))| {
            if *total < min_trades { return None; }
            let wr = *wins as f64 / *total as f64;
            Some((k.clone(), wr))
        })
        .collect();

    if raw.is_empty() {
        return std::collections::HashMap::new();
    }

    // Normalize
    let total_wr: f64 = raw.iter().map(|(_, w)| w).sum();
    if total_wr == 0.0 {
        return std::collections::HashMap::new();
    }
    for (_, w) in &mut raw {
        *w /= total_wr;
    }
    raw.into_iter().collect()
}

// ── Public helpers for strategy_runner integration ────────────────────────────

/// Append a resolved trade to the asset selector stats file.
/// Called automatically by the strategy runner when a trade closes.
pub async fn record_trade(
    workspace_dir: &std::path::Path,
    script: &str,
    symbol: &str,
    window_ts: i64,
    won: bool,
    pnl: f64,
) {
    let path = workspace_dir.join("data").join(FILE_NAME);
    let mut stats: Stats = match tokio::fs::read_to_string(&path).await {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Stats::default(),
    };
    stats.entries.retain(|e| {
        !(e.script == script && e.symbol == symbol && e.window_ts == window_ts)
    });
    stats.entries.push(TradeEntry {
        script: script.to_string(),
        symbol: symbol.to_string(),
        window_ts,
        won,
        pnl,
    });
    let cutoff = chrono::Utc::now().timestamp() - ROLLING_DAYS * 86_400;
    stats.entries.retain(|e| e.window_ts >= cutoff);
    if let Ok(json) = serde_json::to_string_pretty(&stats) {
        if let Some(parent) = path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let _ = tokio::fs::write(&path, json).await;
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn err(msg: &str) -> ToolResult {
    ToolResult {
        success: false,
        output: String::new(),
        error: Some(msg.to_string()),
    }
}

use super::traits::{Tool, ToolResult};
use async_trait::async_trait;
use market_analyzer::screener::MarketData;
use serde_json::json;

const DEFAULT_TOP_N: usize = 20;

/// Fetch live market indicators from the TradingView Screener.
pub struct MarketScanTool;

#[async_trait]
impl Tool for MarketScanTool {
    fn name(&self) -> &str {
        "market_scan"
    }

    fn description(&self) -> &str {
        "Fetch live price, RSI, and MACD indicators for crypto symbols from the TradingView \
        Screener. Use this to scan the market for trading opportunities, identify overbought / \
        oversold conditions (RSI < 30 = oversold, RSI > 70 = overbought), and detect MACD \
        crossovers.\n\
        - If no symbols are provided, fetches the top N pairs by 24h volume live from TradingView \
        (not a hardcoded list).\n\
        - You can specify exact symbols (e.g. [\"BTCUSDT\",\"SOLUSDT\"]) or exchange-qualified \
        ones (e.g. [\"BYBIT:BTCUSDT\"]).\n\
        - Use the `limit` parameter to control how many top-volume pairs to scan (default: 20)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "symbols": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Specific symbols to scan (e.g. [\"BTCUSDT\", \"SOLUSDT\"]). \
                    Supports exchange-qualified format like \"BYBIT:BTCUSDT\". \
                    Leave empty to use top-volume dynamic scan."
                },
                "limit": {
                    "type": "integer",
                    "description": "When no symbols are specified, how many top-volume pairs to fetch (default: 20, max: 50).",
                    "default": 20
                }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let symbol_strings: Vec<String> = args
            .get("symbols")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_uppercase))
                    .collect()
            })
            .unwrap_or_default();

        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).min(50))
            .unwrap_or(DEFAULT_TOP_N);

        let (data, source_label) = if symbol_strings.is_empty() {
            // Dynamic: fetch top-N by volume live from TradingView
            match market_analyzer::screener::fetch_top_by_volume(limit).await {
                Ok(d) => (d, format!("top {} by 24h volume (live)", limit)),
                Err(e) => {
                    // Fallback to static list if network fails
                    tracing::warn!("fetch_top_by_volume failed ({e}), falling back to static list");
                    let fallback = market_analyzer::screener::top_crypto_symbols_fallback();
                    let refs: Vec<&str> = fallback.iter().copied().collect();
                    match market_analyzer::screener::fetch_indicators(&refs).await {
                        Ok(d) => (d, "top 20 (static fallback — network error)".to_string()),
                        Err(e2) => {
                            return Ok(ToolResult {
                                success: false,
                                output: String::new(),
                                error: Some(format!("TradingView screener error: {e2}")),
                            });
                        }
                    }
                }
            }
        } else {
            // User-specified symbols
            let refs: Vec<&str> = symbol_strings.iter().map(String::as_str).collect();
            let label = refs.join(", ");
            match market_analyzer::screener::fetch_indicators(&refs).await {
                Ok(d) => (d, label),
                Err(e) => {
                    return Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some(format!("TradingView screener error: {e}")),
                    });
                }
            }
        };

        if data.is_empty() {
            return Ok(ToolResult {
                success: true,
                output: "No data returned from screener.".into(),
                error: None,
            });
        }

        Ok(ToolResult {
            success: true,
            output: format_results(&data, &source_label),
            error: None,
        })
    }
}

fn format_results(data: &[MarketData], source: &str) -> String {
    let mut lines = vec![
        format!("Market scan — {source}"),
        format!(
            "{:<14} {:>10}  {:>12}  {:>6}  {:>8}  {:>8}  SIGNAL",
            "SYMBOL", "PRICE", "VOLUME 24H", "RSI", "MACD", "SIG LINE"
        ),
        "─".repeat(80),
    ];

    let mut opportunities: Vec<String> = Vec::new();

    for d in data {
        let vol_str = d
            .volume
            .map(|v| format_volume(v))
            .unwrap_or_else(|| "         —".into());
        let rsi_str = d.rsi.map_or("  n/a".into(), |r| format!("{r:>6.1}"));
        let macd_str = d.macd.map_or("     n/a".into(), |m| format!("{m:>8.3}"));
        let sig_str = d
            .macd_signal
            .map_or("     n/a".into(), |s| format!("{s:>8.3}"));

        let signal = classify(&d.rsi, &d.macd, &d.macd_signal);

        lines.push(format!(
            "{:<14} {:>10.2}  {}  {}  {}  {}  {}",
            d.symbol, d.price, vol_str, rsi_str, macd_str, sig_str, signal
        ));

        if signal != "—" {
            opportunities.push(format!("{}: {}", d.symbol, signal));
        }
    }

    lines.push("─".repeat(80));

    if opportunities.is_empty() {
        lines.push("No strong signals at this time.".into());
    } else {
        lines.push("Opportunities:".into());
        for o in &opportunities {
            lines.push(format!("  • {o}"));
        }
    }

    lines.join("\n")
}

fn format_volume(v: f64) -> String {
    if v >= 1_000_000_000.0 {
        format!("{:>9.2}B", v / 1_000_000_000.0)
    } else if v >= 1_000_000.0 {
        format!("{:>9.2}M", v / 1_000_000.0)
    } else if v >= 1_000.0 {
        format!("{:>9.2}K", v / 1_000.0)
    } else {
        format!("{v:>10.2}")
    }
}

fn classify(rsi: &Option<f64>, macd: &Option<f64>, signal: &Option<f64>) -> String {
    let mut tags: Vec<&str> = Vec::new();

    if let Some(r) = rsi {
        if *r < 30.0 {
            tags.push("RSI OVERSOLD ↑");
        } else if *r > 70.0 {
            tags.push("RSI OVERBOUGHT ↓");
        }
    }

    if let (Some(m), Some(s)) = (macd, signal) {
        if m > s && (m - s).abs() > 0.0001 {
            tags.push("MACD bullish cross");
        } else if m < s && (m - s).abs() > 0.0001 {
            tags.push("MACD bearish cross");
        }
    }

    if tags.is_empty() { "—".into() } else { tags.join(", ") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_oversold() {
        assert!(classify(&Some(25.0), &None, &None).contains("OVERSOLD"));
    }

    #[test]
    fn classify_overbought() {
        assert!(classify(&Some(75.0), &None, &None).contains("OVERBOUGHT"));
    }

    #[test]
    fn classify_macd_bullish() {
        assert!(classify(&Some(50.0), &Some(0.05), &Some(0.01)).contains("bullish"));
    }

    #[test]
    fn classify_neutral() {
        assert_eq!(classify(&Some(50.0), &Some(0.01), &Some(0.01)), "—");
    }

    #[test]
    fn format_volume_billions() {
        assert!(format_volume(1_500_000_000.0).contains('B'));
    }

    #[test]
    fn format_volume_millions() {
        assert!(format_volume(25_000_000.0).contains('M'));
    }

    #[tokio::test]
    async fn schema_is_valid() {
        let tool = MarketScanTool;
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["symbols"].is_object());
        assert!(schema["properties"]["limit"].is_object());
    }
}

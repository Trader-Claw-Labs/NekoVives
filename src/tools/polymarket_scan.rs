use super::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;

/// Fetch active Polymarket prediction markets with YES/NO prices, volume, and expiry.
pub struct PolymarketScanTool;

#[async_trait]
impl Tool for PolymarketScanTool {
    fn name(&self) -> &str {
        "polymarket_scan"
    }

    fn description(&self) -> &str {
        "Fetch active Polymarket prediction markets including YES price, NO price, 24h volume, \
        liquidity, and expiration date. Use this to monitor markets, detect price movements, \
        find high-volume opportunities, and check market conditions for prediction market trading. \
        Supports filtering by minimum volume, minimum liquidity, and free-text search query. \
        Returns YES/NO prices as probabilities (0.0-1.0), e.g. YES=0.72 means 72% implied probability. \
        ALWAYS use this tool for any Polymarket-related scanning or monitoring -- never use market_scan \
        for Polymarket data."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Optional search query to filter markets by question text (e.g. 'BTC', 'election', 'weather')."
                },
                "min_volume": {
                    "type": "number",
                    "description": "Minimum 24h volume in USDC to filter results (e.g. 10000 for $10k+)."
                },
                "min_liquidity": {
                    "type": "number",
                    "description": "Minimum liquidity in USDC to filter results."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of markets to return (default: 20, max: 50).",
                    "default": 20
                },
                "include_prices": {
                    "type": "boolean",
                    "description": "Whether to fetch live YES/NO prices from the CLOB API (default: true). Set false for faster results without prices.",
                    "default": true
                }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        use polymarket_trader::markets::{list_markets, get_market_price, MarketFilter};

        let query = args.get("query").and_then(|v| v.as_str()).map(str::to_string);
        let min_volume = args.get("min_volume").and_then(|v| v.as_f64());
        let min_liquidity = args.get("min_liquidity").and_then(|v| v.as_f64());
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20).min(50) as usize;
        let include_prices = args.get("include_prices").and_then(|v| v.as_bool()).unwrap_or(true);

        let filter = MarketFilter {
            query,
            min_volume_usdc: min_volume,
            min_liquidity_usdc: min_liquidity,
            active_only: true,
            limit: Some(limit),
            ..Default::default()
        };

        let markets = match list_markets(filter).await {
            Ok(m) => m,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Polymarket API error: {e}")),
                });
            }
        };

        if markets.is_empty() {
            return Ok(ToolResult {
                success: true,
                output: "No active Polymarket markets found matching the criteria.".into(),
                error: None,
            });
        }

        let now = chrono::Utc::now();
        let mut rows: Vec<serde_json::Value> = Vec::with_capacity(markets.len());

        for m in &markets {
            let (yes_price, no_price) = if include_prices {
                let yes = get_market_price(&m.yes_token_id).await.unwrap_or(f64::NAN);
                let no = get_market_price(&m.no_token_id).await.unwrap_or(f64::NAN);
                (yes, no)
            } else {
                (f64::NAN, f64::NAN)
            };

            let days_to_expiry = m.end_date_iso.as_deref().and_then(|s| {
                chrono::DateTime::parse_from_rfc3339(s).ok().map(|end| {
                    end.signed_duration_since(now).num_days()
                })
            });

            let mut row = json!({
                "question": m.question,
                "slug": m.slug,
                "volume_usdc": (m.volume * 100.0).round() / 100.0,
                "liquidity_usdc": (m.liquidity * 100.0).round() / 100.0,
                "category": m.category,
                "end_date": m.end_date_iso,
                "days_to_expiry": days_to_expiry,
            });

            if include_prices && yes_price.is_finite() {
                row["yes_price"] = json!((yes_price * 1000.0).round() / 1000.0);
                row["no_price"] = json!((no_price * 1000.0).round() / 1000.0);
                row["yes_pct"] = json!(format!("{:.1}%", yes_price * 100.0));
            }

            rows.push(row);
        }

        let output = serde_json::to_string_pretty(&json!({ "markets": rows, "count": rows.len() }))
            .unwrap_or_else(|_| format!("Found {} markets", rows.len()));

        Ok(ToolResult {
            success: true,
            output,
            error: None,
        })
    }
}

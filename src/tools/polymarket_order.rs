use super::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use crate::config::Config;
use crate::security::SecurityPolicy;
use parking_lot::Mutex;

pub struct PolymarketOrderTool {
    pub config: Arc<Mutex<Config>>,
    pub security: Arc<SecurityPolicy>,
}

#[async_trait]
impl Tool for PolymarketOrderTool {
    fn name(&self) -> &str { "polymarket_order" }

    fn description(&self) -> &str {
        "Place, cancel, or list orders on Polymarket prediction markets. \
        Supports market and limit orders for YES/NO tokens. \
        ALWAYS confirm the token_id, side, amount, and price with the user before placing. \
        Requires Polymarket credentials to be configured."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["buy", "sell", "cancel", "list_open"], "description": "Action to perform" },
                "token_id": { "type": "string", "description": "YES or NO token ID (condition token ID from Polymarket)" },
                "side": { "type": "string", "enum": ["buy", "sell"], "description": "Order side" },
                "amount_usdc": { "type": "number", "description": "Amount in USDC to spend (for buy) or size to sell" },
                "price": { "type": "number", "description": "Limit price 0-1 (e.g. 0.65 = 65 cents). Leave out for market order" },
                "order_id": { "type": "string", "description": "Order ID to cancel (for cancel action)" }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        if !self.security.can_act() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Security policy: read-only mode".into()),
            });
        }

        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list_open");

        // Get polymarket credentials from config
        let creds_opt: Option<polymarket_trader::auth::PolyCredentials> = {
            let cfg = self.config.lock();
            let pm = &cfg.polymarket;
            match (&pm.api_key, &pm.secret, &pm.passphrase) {
                (Some(key), Some(secret), Some(pass))
                    if !key.is_empty() && !secret.is_empty() && !pass.is_empty() =>
                {
                    Some(polymarket_trader::auth::PolyCredentials {
                        api_key: key.clone(),
                        secret: secret.clone(),
                        passphrase: pass.clone(),
                        wallet_address: pm.wallet_address.clone().unwrap_or_default().to_lowercase(),
                        private_key: pm.private_key.clone().filter(|k| !k.is_empty()),
                        is_builder: pm.is_builder.unwrap_or(false),
                        proxy_address: pm.proxy_address.clone().filter(|k| !k.is_empty()).map(|s| s.to_lowercase()),
                        signature_type: pm.signature_type.clone().filter(|k| !k.is_empty()),
                    })
                }
                _ => None,
            }
        };

        if creds_opt.is_none() && action != "list_open" {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Polymarket credentials not configured. Go to Settings → Polymarket to configure api_key, secret, and passphrase.".into()),
            });
        }

        match action {
            "list_open" => {
                let creds = match creds_opt {
                    Some(c) => c,
                    None => return Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some("Polymarket credentials not configured.".into()),
                    }),
                };
                let client = polymarket_trader::orders::ClobClient::new(creds);
                match client.get_open_orders().await {
                    Ok(orders) => {
                        if orders.is_empty() {
                            Ok(ToolResult { success: true, output: "No open orders on Polymarket.".into(), error: None })
                        } else {
                            let lines: Vec<String> = orders.iter().map(|o| {
                                format!("  ID: {} | {} {} @ {} | status: {}", o.id, o.side, o.size, o.price, o.status)
                            }).collect();
                            Ok(ToolResult { success: true, output: format!("Open Polymarket orders:\n{}", lines.join("\n")), error: None })
                        }
                    }
                    Err(e) => Ok(ToolResult { success: false, output: String::new(), error: Some(format!("Failed to fetch orders: {e}")) })
                }
            }
            "cancel" => {
                let order_id = match args.get("order_id").and_then(|v| v.as_str()) {
                    Some(id) => id,
                    None => return Ok(ToolResult { success: false, output: String::new(), error: Some("order_id is required for cancel action".into()) }),
                };
                let creds = creds_opt.unwrap();
                let client = polymarket_trader::orders::ClobClient::new(creds);
                match client.cancel_order(order_id).await {
                    Ok(()) => Ok(ToolResult { success: true, output: format!("Order {} cancelled successfully.", order_id), error: None }),
                    Err(e) => Ok(ToolResult { success: false, output: String::new(), error: Some(format!("Failed to cancel order: {e}")) })
                }
            }
            "buy" | "sell" => {
                let token_id = match args.get("token_id").and_then(|v| v.as_str()) {
                    Some(id) => id,
                    None => return Ok(ToolResult { success: false, output: String::new(), error: Some("token_id is required".into()) }),
                };
                let amount = match args.get("amount_usdc").and_then(|v| v.as_f64()) {
                    Some(a) => a,
                    None => return Ok(ToolResult { success: false, output: String::new(), error: Some("amount_usdc is required".into()) }),
                };
                let price_opt = args.get("price").and_then(|v| v.as_f64());
                let creds = creds_opt.unwrap();
                let client = polymarket_trader::orders::ClobClient::new(creds);

                let side = if action == "buy" {
                    polymarket_trader::orders::Side::Buy
                } else {
                    polymarket_trader::orders::Side::Sell
                };

                let result = if let Some(price) = price_opt {
                    // Limit order: size is amount_usdc / price
                    let size = amount / price;
                    client.create_limit_order(token_id, side, price, size).await
                } else {
                    // Market order: amount is the notional to trade.  worst_price
                    // values outside [0.01, 0.99] tell the SDK to calculate the
                    // real price from the orderbook instead of using a fixed slippage cap.
                    let worst_price = 0.0_f64;
                    client.create_market_order(token_id, side, amount, worst_price).await
                };

                match result {
                    Ok(order_resp) => Ok(ToolResult {
                        success: true,
                        output: format!("Order placed successfully. Order ID: {} | Status: {}", order_resp.order_id, order_resp.status),
                        error: None,
                    }),
                    Err(e) => Ok(ToolResult { success: false, output: String::new(), error: Some(format!("Failed to place order: {e}")) })
                }
            }
            _ => Ok(ToolResult { success: false, output: String::new(), error: Some(format!("Unknown action: {}", action)) })
        }
    }
}

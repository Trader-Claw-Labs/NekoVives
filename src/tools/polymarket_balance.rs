use super::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use crate::config::Config;

pub struct PolymarketBalanceTool {
    pub config: Arc<Config>,
}

#[async_trait]
impl Tool for PolymarketBalanceTool {
    fn name(&self) -> &str {
        "polymarket_balance"
    }

    fn description(&self) -> &str {
        "Fetch the USDC balance of the configured Polymarket wallet. \
        Queries the Polymarket CLOB API first (reflects available trading balance), \
        then falls back to the Polygon RPC balance. \
        Use this whenever the user asks about their Polymarket balance, available funds, \
        or how much USDC they have on Polymarket. \
        Requires Polymarket credentials to be configured in Settings."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let creds_opt: Option<polymarket_trader::auth::PolyCredentials> = {
            let pm = &self.config.polymarket;
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
                    })
                }
                _ => None,
            }
        };

        let creds = match creds_opt {
            Some(c) => c,
            None => return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(
                    "Polymarket credentials not configured. Go to Settings → Polymarket to configure api_key, secret, and passphrase.".into(),
                ),
            }),
        };

        let wallet = creds.proxy_address.clone()
            .filter(|a| !a.is_empty())
            .unwrap_or_else(|| creds.wallet_address.clone());

        let client = polymarket_trader::orders::ClobClient::new(creds);

        // Try CLOB API balance first (reflects the tradeable balance seen by the exchange)
        let (balance, source) = match client.get_api_balance().await {
            Ok(b) => (b, "CLOB API"),
            Err(clob_err) => {
                tracing::warn!("polymarket_balance: CLOB balance failed ({}), trying Polygon RPC", clob_err);
                match client.get_balance().await {
                    Ok(b) => (b, "Polygon RPC"),
                    Err(rpc_err) => {
                        return Ok(ToolResult {
                            success: false,
                            output: String::new(),
                            error: Some(format!(
                                "Could not fetch Polymarket balance. CLOB error: {}. RPC error: {}",
                                clob_err, rpc_err
                            )),
                        });
                    }
                }
            }
        };

        Ok(ToolResult {
            success: true,
            output: format!(
                "Polymarket USDC balance: ${:.2} (source: {}, wallet: {})",
                balance, source, wallet
            ),
            error: None,
        })
    }
}

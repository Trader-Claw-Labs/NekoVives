use super::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;

// Well-known token mint addresses on Solana
const SOL_MINT: &str = "So11111111111111111111111111111111111111112";
const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const USDT_MINT: &str = "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB";
const BONK_MINT: &str = "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263";

fn resolve_solana_mint(token: &str) -> Option<&'static str> {
    match token.to_uppercase().as_str() {
        "SOL" => Some(SOL_MINT),
        "USDC" => Some(USDC_MINT),
        "USDT" => Some(USDT_MINT),
        "BONK" => Some(BONK_MINT),
        _ => None,
    }
}

fn token_decimals(mint: &str) -> u32 {
    match mint {
        SOL_MINT => 9,
        USDC_MINT | USDT_MINT => 6,
        _ => 9,
    }
}

// ── Jupiter quote (returns raw JSON so it can be forwarded to /v6/swap) ──

async fn get_jupiter_quote_raw(
    input_mint: &str,
    output_mint: &str,
    amount_lamports: u64,
) -> anyhow::Result<Value> {
    let url = format!(
        "https://quote-api.jup.ag/v6/quote?inputMint={input_mint}&outputMint={output_mint}&amount={amount_lamports}&slippageBps=50"
    );
    let resp = reqwest::Client::new()
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await?
        .error_for_status()?
        .json::<Value>()
        .await?;
    Ok(resp)
}

fn extract_quote_display(quote: &Value, from_token: &str, to_token: &str, amount: f64, out_decimals: u32) -> String {
    let out_amount_raw = quote.get("outAmount")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let out_amount = out_amount_raw as f64 / 10_f64.powi(out_decimals as i32);
    let price_impact = quote.get("priceImpactPct").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let route_labels: Vec<String> = quote
        .get("routePlan")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|step| {
                    let label = step.pointer("/swapInfo/label")
                        .and_then(|v| v.as_str())
                        .unwrap_or_else(|| {
                            step.pointer("/swapInfo/ammKey")
                                .and_then(|v| v.as_str())
                                .unwrap_or("?")
                        });
                    Some(label.to_string())
                })
                .collect()
        })
        .unwrap_or_default();

    format!(
        "Sell:         {:.6} {from_token}\n\
         Receive:      {:.6} {to_token}\n\
         Price impact: {:.3}%\n\
         Route:        {}",
        amount,
        out_amount,
        price_impact,
        if route_labels.is_empty() { "direct".to_string() } else { route_labels.join(" → ") },
    )
}

// ── Wallet reader ────────────────────────────────────────────────────

#[derive(Deserialize)]
struct DiskWallet {
    chain: String,
    address: String,
    label: String,
    encrypted_key_b64: String,
}

fn find_wallet(config_path: &std::path::Path, chain: &str, hint: Option<&str>) -> Option<DiskWallet> {
    let wallets_path = config_path.parent().unwrap_or(config_path).join("wallets.json");
    let data = std::fs::read_to_string(&wallets_path).ok()?;
    let entries: Vec<DiskWallet> = serde_json::from_str(&data).ok()?;
    entries.into_iter().find(|w| {
        let chain_ok = w.chain.to_lowercase() == chain.to_lowercase();
        let hint_ok = hint.map_or(true, |h| {
            w.address.to_lowercase().contains(&h.to_lowercase())
                || w.label.to_lowercase().contains(&h.to_lowercase())
        });
        chain_ok && hint_ok
    })
}

// ── Tool ─────────────────────────────────────────────────────────────

pub struct TradeSwapTool {
    config_path: PathBuf,
}

impl TradeSwapTool {
    pub fn new(config_path: PathBuf) -> Self {
        Self { config_path }
    }
}

#[async_trait]
impl Tool for TradeSwapTool {
    fn name(&self) -> &str {
        "trade_swap"
    }

    fn description(&self) -> &str {
        "Execute a token swap on Solana using Jupiter aggregator. \
        Before calling this tool you MUST:\n\
        1. Call `wallet_balance` to confirm a Solana wallet exists with sufficient funds.\n\
        2. Confirm the exact amount and output token with the user.\n\
        3. Ask for the wallet password to decrypt the signing key.\n\
        This tool validates all parameters, fetches a live Jupiter quote showing \
        the expected output and price impact, and requires explicit user confirmation \
        before signing the transaction."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "from_token": {
                    "type": "string",
                    "description": "Token to sell, e.g. 'SOL', 'BONK', or a full mint address"
                },
                "to_token": {
                    "type": "string",
                    "description": "Token to receive, e.g. 'USDC', 'USDT', 'SOL', or a full mint address"
                },
                "amount": {
                    "type": "number",
                    "description": "Amount of from_token to sell (in human units, e.g. 1.5 for 1.5 SOL)"
                },
                "wallet_address": {
                    "type": "string",
                    "description": "Wallet address or label to use (optional — uses first Solana wallet if omitted)"
                },
                "wallet_password": {
                    "type": "string",
                    "description": "Password to decrypt the wallet signing key. Required to sign the transaction."
                },
                "confirmed": {
                    "type": "boolean",
                    "description": "Set to true only after the user has reviewed the quote and explicitly confirmed",
                    "default": false
                }
            },
            "required": ["from_token", "to_token", "amount"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let from_token = args.get("from_token").and_then(|v| v.as_str()).unwrap_or("").to_uppercase();
        let to_token = args.get("to_token").and_then(|v| v.as_str()).unwrap_or("").to_uppercase();
        let amount: f64 = args.get("amount").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let wallet_hint = args.get("wallet_address").and_then(|v| v.as_str());
        let wallet_password = args.get("wallet_password").and_then(|v| v.as_str());
        let confirmed = args.get("confirmed").and_then(|v| v.as_bool()).unwrap_or(false);

        // ── Validate params ──────────────────────────────────────────
        if from_token.is_empty() || to_token.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Both from_token and to_token are required.".into()),
            });
        }
        if amount <= 0.0 {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Amount must be greater than 0.".into()),
            });
        }
        if from_token == to_token {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("from_token and to_token are the same ({from_token}).")),
            });
        }

        // ── Resolve mint addresses ───────────────────────────────────
        let input_mint = if from_token.len() > 10 {
            from_token.clone()  // already a mint address
        } else {
            match resolve_solana_mint(&from_token) {
                Some(m) => m.to_string(),
                None => return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!(
                        "Unknown token '{from_token}'. Supported: SOL, USDC, USDT, BONK, \
                        or provide the full mint address."
                    )),
                }),
            }
        };
        let output_mint = if to_token.len() > 10 {
            to_token.clone()
        } else {
            match resolve_solana_mint(&to_token) {
                Some(m) => m.to_string(),
                None => return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!(
                        "Unknown token '{to_token}'. Supported: SOL, USDC, USDT, BONK, \
                        or provide the full mint address."
                    )),
                }),
            }
        };

        // ── Check wallet exists ──────────────────────────────────────
        let wallet = find_wallet(&self.config_path, "solana", wallet_hint);
        let wallet = match wallet {
            Some(w) => w,
            None => {
                let hint_msg = wallet_hint.map(|h| format!(" matching '{h}'")).unwrap_or_default();
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!(
                        "No Solana wallet found{hint_msg}. \
                        Create one on the /wallets page or ask me to create one. \
                        Then use wallet_balance to verify it has funds."
                    )),
                });
            }
        };

        // ── Fetch Jupiter quote ──────────────────────────────────────
        let decimals = token_decimals(&input_mint);
        let amount_lamports = (amount * 10_f64.powi(decimals as i32)) as u64;

        let quote_raw = match get_jupiter_quote_raw(&input_mint, &output_mint, amount_lamports).await {
            Ok(q) => q,
            Err(e) => return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Jupiter quote failed: {e}")),
            }),
        };

        let out_decimals = token_decimals(&output_mint);
        let quote_detail = extract_quote_display(&quote_raw, &from_token, &to_token, amount, out_decimals);

        let quote_summary = format!(
            "Jupiter Quote\n\
            ─────────────────────────────────\n\
            Wallet:       {} ({})\n\
            {quote_detail}\n\
            ─────────────────────────────────",
            wallet.label,
            wallet.address,
        );

        // ── Require explicit confirmation ────────────────────────────
        if !confirmed {
            return Ok(ToolResult {
                success: true,
                output: format!(
                    "{quote_summary}\n\n\
                    ⚠️  Trade not yet executed. Show the user this quote and ask:\n\
                    1. Do they confirm this trade?\n\
                    2. What is their wallet password to sign the transaction?\n\
                    Then call trade_swap again with confirmed=true and wallet_password set."
                ),
                error: None,
            });
        }

        // ── Check password was provided ──────────────────────────────
        let password = match wallet_password {
            Some(p) if !p.is_empty() => p,
            _ => return Ok(ToolResult {
                success: false,
                output: format!("{quote_summary}\n\nTrade confirmed but wallet_password is required to sign."),
                error: Some("wallet_password is required to sign the transaction.".into()),
            }),
        };

        // ── Decrypt private key ──────────────────────────────────────
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let encrypted_key = match STANDARD.decode(&wallet.encrypted_key_b64) {
            Ok(k) => k,
            Err(e) => return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Failed to decode wallet key: {e}")),
            }),
        };

        let privkey_vec = match wallet_manager::solana::export_private_key(&encrypted_key, password) {
            Ok(k) => k,
            Err(_) => return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Incorrect wallet password. Transaction not signed.".into()),
            }),
        };

        let privkey_arr: [u8; 32] = match privkey_vec.try_into() {
            Ok(arr) => arr,
            Err(_) => return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Unexpected private key length after decryption.".into()),
            }),
        };

        // ── Get swap transaction from Jupiter, sign, broadcast ───────
        let trader = solana_trader::SolanaTrader::new(None);

        let swap_tx_b64 = match trader.get_swap_transaction(&quote_raw, &wallet.address).await {
            Ok(tx) => tx,
            Err(e) => return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Jupiter /v6/swap failed: {e}")),
            }),
        };

        let signed_tx_b64 = match solana_trader::SolanaTrader::sign_transaction(&swap_tx_b64, &privkey_arr) {
            Ok(tx) => tx,
            Err(e) => return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Transaction signing failed: {e}")),
            }),
        };

        match trader.broadcast_transaction(&signed_tx_b64).await {
            Ok(sig) => Ok(ToolResult {
                success: true,
                output: format!(
                    "{quote_summary}\n\n\
                    ✅ Transaction submitted!\n\
                    Signature: {sig}\n\
                    Explorer:  https://solscan.io/tx/{sig}"
                ),
                error: None,
            }),
            Err(e) => Ok(ToolResult {
                success: false,
                output: quote_summary,
                error: Some(format!("Broadcast failed: {e}")),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_tool() -> (TempDir, TradeSwapTool) {
        let tmp = TempDir::new().unwrap();
        let tool = TradeSwapTool::new(tmp.path().join("config.toml"));
        (tmp, tool)
    }

    #[tokio::test]
    async fn missing_params() {
        let (_tmp, tool) = make_tool();
        let result = tool.execute(json!({ "amount": 1.0 })).await.unwrap();
        assert!(!result.success);
    }

    #[tokio::test]
    async fn zero_amount() {
        let (_tmp, tool) = make_tool();
        let result = tool
            .execute(json!({ "from_token": "SOL", "to_token": "USDC", "amount": 0 }))
            .await
            .unwrap();
        assert!(!result.success);
    }

    #[tokio::test]
    async fn same_token() {
        let (_tmp, tool) = make_tool();
        let result = tool
            .execute(json!({ "from_token": "SOL", "to_token": "SOL", "amount": 1.0 }))
            .await
            .unwrap();
        assert!(!result.success);
    }

    #[tokio::test]
    async fn unknown_token() {
        let (_tmp, tool) = make_tool();
        let result = tool
            .execute(json!({ "from_token": "XXXX", "to_token": "USDC", "amount": 1.0 }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap_or("").contains("Unknown token"));
    }

    #[tokio::test]
    async fn no_wallet_registered() {
        let (_tmp, tool) = make_tool();
        let result = tool
            .execute(json!({ "from_token": "SOL", "to_token": "USDC", "amount": 1.0 }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap_or("").contains("No Solana wallet"));
    }
}

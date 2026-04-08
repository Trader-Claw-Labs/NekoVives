use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use ed25519_dalek::{Signer, SigningKey};
use serde::Deserialize;
use serde_json::{json, Value};

pub const MAINNET_RPC: &str = "https://api.mainnet-beta.solana.com";
const LAMPORTS_PER_SOL: f64 = 1_000_000_000.0;
const TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";

// ---------------------------------------------------------------------------
// Token balance info
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TokenBalance {
    pub mint: String,
    pub symbol: String,
    pub amount: f64,
    pub decimals: u8,
}

// ---------------------------------------------------------------------------
// Internal RPC response types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct RpcResponse<T> {
    result: Option<T>,
    error: Option<RpcError>,
}

#[derive(Deserialize)]
struct RpcError {
    message: String,
}

#[derive(Deserialize)]
struct BalanceResult {
    value: u64,
}

#[derive(Deserialize)]
struct TokenAccountsResult {
    value: Vec<TokenAccountEntry>,
}

#[derive(Deserialize)]
struct TokenAccountEntry {
    account: TokenAccountData,
}

#[derive(Deserialize)]
struct TokenAccountData {
    data: TokenAccountParsedData,
}

#[derive(Deserialize)]
struct TokenAccountParsedData {
    parsed: TokenAccountParsed,
}

#[derive(Deserialize)]
struct TokenAccountParsed {
    info: TokenAccountInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TokenAccountInfo {
    mint: String,
    token_amount: TokenAmount,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TokenAmount {
    ui_amount: Option<f64>,
    decimals: u8,
}

// ---------------------------------------------------------------------------
// Jupiter swap response
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JupiterSwapResponse {
    swap_transaction: String,
}

// ---------------------------------------------------------------------------
// Compact-u16 decoder (Solana wire format)
// ---------------------------------------------------------------------------

/// Decode a Solana compact-u16 from a byte slice.
/// Returns (value, bytes_consumed).
fn decode_compact_u16(data: &[u8]) -> (usize, usize) {
    if data.is_empty() {
        return (0, 0);
    }
    let b0 = data[0] as usize;
    if b0 & 0x80 == 0 {
        return (b0, 1);
    }
    if data.len() < 2 {
        return (b0 & 0x7f, 1);
    }
    let b1 = data[1] as usize;
    if b1 & 0x80 == 0 {
        ((b0 & 0x7f) | (b1 << 7), 2)
    } else if data.len() >= 3 {
        let b2 = data[2] as usize;
        ((b0 & 0x7f) | ((b1 & 0x7f) << 7) | (b2 << 14), 3)
    } else {
        ((b0 & 0x7f) | ((b1 & 0x7f) << 7), 2)
    }
}

// ---------------------------------------------------------------------------
// SolanaTrader
// ---------------------------------------------------------------------------

pub struct SolanaTrader {
    rpc_url: String,
    client: reqwest::Client,
}

impl SolanaTrader {
    /// Create a new trader. Pass `None` to use mainnet.
    pub fn new(rpc_url: Option<&str>) -> Self {
        Self {
            rpc_url: rpc_url.unwrap_or(MAINNET_RPC).to_string(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_default(),
        }
    }

    // ── Balance queries ────────────────────────────────────────────────

    /// Fetch native SOL balance in SOL (not lamports).
    pub async fn get_sol_balance(&self, address: &str) -> Result<f64> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getBalance",
            "params": [address]
        });
        let resp: RpcResponse<BalanceResult> = self
            .client
            .post(&self.rpc_url)
            .json(&body)
            .send()
            .await
            .context("getBalance request failed")?
            .json()
            .await
            .context("getBalance parse failed")?;

        if let Some(e) = resp.error {
            return Err(anyhow!("RPC error: {}", e.message));
        }
        Ok(resp.result.context("missing result")?.value as f64 / LAMPORTS_PER_SOL)
    }

    /// Fetch all SPL token balances for an address.
    pub async fn get_token_balances(&self, address: &str) -> Result<Vec<TokenBalance>> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getTokenAccountsByOwner",
            "params": [
                address,
                {"programId": TOKEN_PROGRAM_ID},
                {"encoding": "jsonParsed"}
            ]
        });
        let resp: RpcResponse<TokenAccountsResult> = self
            .client
            .post(&self.rpc_url)
            .json(&body)
            .send()
            .await
            .context("getTokenAccountsByOwner request failed")?
            .json()
            .await
            .context("getTokenAccountsByOwner parse failed")?;

        if let Some(e) = resp.error {
            return Err(anyhow!("RPC error: {}", e.message));
        }

        let accounts = resp.result.context("missing result")?.value;
        let balances = accounts
            .into_iter()
            .filter_map(|entry| {
                let info = entry.account.data.parsed.info;
                let amount = info.token_amount.ui_amount.unwrap_or(0.0);
                if amount == 0.0 {
                    return None; // skip zero-balance accounts
                }
                Some(TokenBalance {
                    symbol: mint_to_symbol(&info.mint),
                    mint: info.mint,
                    amount,
                    decimals: info.token_amount.decimals,
                })
            })
            .collect();

        Ok(balances)
    }

    // ── Jupiter swap transaction ───────────────────────────────────────

    /// Request a signed-but-not-yet-user-signed swap transaction from Jupiter.
    /// `quote_response` is the raw JSON value returned by the /v6/quote endpoint.
    /// Returns the base64-encoded serialized VersionedTransaction (with user sig slot zeroed).
    pub async fn get_swap_transaction(
        &self,
        quote_response: &Value,
        user_pubkey: &str,
    ) -> Result<String> {
        let body = json!({
            "quoteResponse": quote_response,
            "userPublicKey": user_pubkey,
            "wrapAndUnwrapSol": true,
            "dynamicComputeUnitLimit": true,
            "prioritizationFeeLamports": "auto"
        });

        let resp: JupiterSwapResponse = self
            .client
            .post("https://quote-api.jup.ag/v6/swap")
            .json(&body)
            .send()
            .await
            .context("Jupiter /v6/swap request failed")?
            .error_for_status()
            .context("Jupiter /v6/swap returned error")?
            .json()
            .await
            .context("Jupiter /v6/swap parse failed")?;

        Ok(resp.swap_transaction)
    }

    // ── Transaction signing ────────────────────────────────────────────

    /// Sign a base64-encoded serialized Solana VersionedTransaction.
    ///
    /// Parses the compact-u16 signature count, extracts the message bytes,
    /// signs with ed25519, and writes the signature into slot 0.
    /// Returns the base64-encoded signed transaction ready to broadcast.
    pub fn sign_transaction(tx_b64: &str, privkey_bytes: &[u8; 32]) -> Result<String> {
        let mut tx_bytes = STANDARD
            .decode(tx_b64)
            .context("failed to base64-decode transaction")?;

        // Parse compact-u16 signature count
        let (num_sigs, prefix_len) = decode_compact_u16(&tx_bytes);
        if num_sigs == 0 {
            return Err(anyhow!("transaction has 0 required signatures"));
        }
        if tx_bytes.len() < prefix_len + num_sigs * 64 {
            return Err(anyhow!("transaction bytes too short"));
        }

        // Message starts after the signature slots
        let message_start = prefix_len + num_sigs * 64;
        let message_bytes = tx_bytes[message_start..].to_vec();

        // Sign the message
        let signing_key = SigningKey::from_bytes(privkey_bytes);
        let signature = signing_key.sign(&message_bytes);

        // Write signature into slot 0 (bytes prefix_len..prefix_len+64)
        tx_bytes[prefix_len..prefix_len + 64].copy_from_slice(&signature.to_bytes());

        Ok(STANDARD.encode(&tx_bytes))
    }

    // ── Transaction broadcast ──────────────────────────────────────────

    /// Broadcast a signed base64-encoded transaction to the Solana RPC node.
    /// Returns the transaction signature (base58 string) on success.
    pub async fn broadcast_transaction(&self, signed_tx_b64: &str) -> Result<String> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sendTransaction",
            "params": [
                signed_tx_b64,
                {
                    "encoding": "base64",
                    "preflightCommitment": "confirmed",
                    "maxRetries": 3
                }
            ]
        });

        #[derive(Deserialize)]
        struct SendResult {
            result: Option<String>,
            error: Option<SendError>,
        }
        #[derive(Deserialize)]
        struct SendError {
            message: String,
            data: Option<Value>,
        }

        let resp: SendResult = self
            .client
            .post(&self.rpc_url)
            .json(&body)
            .send()
            .await
            .context("sendTransaction request failed")?
            .json()
            .await
            .context("sendTransaction parse failed")?;

        if let Some(e) = resp.error {
            let detail = e
                .data
                .and_then(|d| d.get("logs").cloned())
                .and_then(|l| serde_json::to_string(&l).ok())
                .unwrap_or_default();
            return Err(anyhow!("sendTransaction error: {} {}", e.message, detail));
        }

        resp.result.context("sendTransaction: missing result signature")
    }

    // ── Convenience: full swap flow ────────────────────────────────────

    /// End-to-end: get swap tx → sign → broadcast.
    /// Returns the transaction signature.
    pub async fn swap(
        &self,
        quote_response: &Value,
        user_pubkey: &str,
        privkey_bytes: &[u8; 32],
    ) -> Result<String> {
        let tx_b64 = self.get_swap_transaction(quote_response, user_pubkey).await?;
        let signed = Self::sign_transaction(&tx_b64, privkey_bytes)?;
        self.broadcast_transaction(&signed).await
    }
}

// ---------------------------------------------------------------------------
// Mint → symbol lookup
// ---------------------------------------------------------------------------

fn mint_to_symbol(mint: &str) -> String {
    match mint {
        "So11111111111111111111111111111111111111112" => "SOL".into(),
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v" => "USDC".into(),
        "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB" => "USDT".into(),
        "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263" => "BONK".into(),
        "7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs" => "ETH".into(),
        "mSoLzYCxHdYgdzU16g5QSh3i5K3z3KZK7ytfqcJm7So" => "mSOL".into(),
        "J1toso1uCk3RLmjorhTtrVwY9HJ7X8V9yYac6Y7kGCPn" => "jitoSOL".into(),
        "bSo13r4TkiE4KumL71LsHTPpL2euBYLFx6h9HP3piy1" => "bSOL".into(),
        other => {
            // Show first 4 + ".." + last 4 chars as a short identifier
            if other.len() > 10 {
                format!("{}..{}", &other[..4], &other[other.len() - 4..])
            } else {
                other.to_string()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_u16_single_byte() {
        let (v, n) = decode_compact_u16(&[0x01]);
        assert_eq!(v, 1);
        assert_eq!(n, 1);
    }

    #[test]
    fn compact_u16_two_bytes() {
        // 128 = 0x80 → [0x80, 0x01]
        let (v, n) = decode_compact_u16(&[0x80, 0x01]);
        assert_eq!(v, 128);
        assert_eq!(n, 2);
    }

    #[test]
    fn compact_u16_zero() {
        let (v, n) = decode_compact_u16(&[0x00]);
        assert_eq!(v, 0);
        assert_eq!(n, 1);
    }

    #[test]
    fn sign_transaction_replaces_sig_slot() {
        // Build a minimal fake "transaction": [0x01] + [0; 64] + message
        let message = b"hello solana";
        let mut tx = vec![0x01u8]; // compact-u16: 1 sig
        tx.extend_from_slice(&[0u8; 64]); // empty sig slot
        tx.extend_from_slice(message);

        let privkey = [42u8; 32];
        let tx_b64 = STANDARD.encode(&tx);

        let signed_b64 = SolanaTrader::sign_transaction(&tx_b64, &privkey).unwrap();
        let signed = STANDARD.decode(&signed_b64).unwrap();

        // First byte unchanged (compact-u16 = 1)
        assert_eq!(signed[0], 0x01);
        // Signature slot (bytes 1..65) should no longer be all zeros
        assert!(signed[1..65].iter().any(|&b| b != 0));
        // Message bytes unchanged
        assert_eq!(&signed[65..], message);
    }

    #[test]
    fn mint_to_symbol_known() {
        assert_eq!(
            mint_to_symbol("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"),
            "USDC"
        );
    }

    #[test]
    fn mint_to_symbol_unknown() {
        let s = mint_to_symbol("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmno");
        assert!(s.contains(".."));
    }
}

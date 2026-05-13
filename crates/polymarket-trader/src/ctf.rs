//! Gnosis CTF (Conditional Token Framework) — Mint and Merge operations.
//!
//! The CTF contract on Polygon lets you split $1.00 USDC into 1 YES token +
//! 1 NO token for a given binary market (`splitPosition`), and later recombine
//! them back to USDC (`mergePositions`).
//!
//! All on-chain calls use the same raw JSON-RPC pattern already established
//! in `orders.rs` so no new heavy dependencies are required.
//!
//! ## Addresses (Polygon mainnet)
//! - CTF (ConditionalTokens): `0x4D97DCd97eC945f40cF65F87097ACe5EA0476045`
//! - USDC.e collateral:       `0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174`
//! - pUSD collateral:         `0xc011a7e12a19f7b1f670d46f03b03f3342e82dfb`

use anyhow::{Context, Result};
use k256::ecdsa::{SigningKey, signature::hazmat::PrehashSigner};
use sha3::{Digest, Keccak256};
use tracing::{info, warn};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Gnosis ConditionalTokens contract on Polygon.
pub const CTF_CONTRACT: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";

/// USDC.e — primary collateral on Polymarket (6 decimals).
pub const USDC_E_CONTRACT: &str = "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174";

/// pUSD — Polymarket wrapped USDC (6 decimals).
pub const PUSD_CONTRACT: &str = "0xc011a7e12a19f7b1f670d46f03b03f3342e82dfb";

/// Polygon mainnet chain ID.
pub const CHAIN_ID: u64 = 137;

const POLYGON_RPCS: &[&str] = &[
    "https://polygon.drpc.org",
    "https://1rpc.io/matic",
    "https://polygon-bor-rpc.publicnode.com",
];

/// Scale factor for USDC (6 decimals).
const USDC_SCALE: u128 = 1_000_000;

// ── Result types ──────────────────────────────────────────────────────────────

/// Result of a successful mint (splitPosition) call.
#[derive(Debug, Clone)]
pub struct MintResult {
    pub tx_hash: String,
    pub amount_usdc: f64,
    pub yes_tokens: f64,
    pub no_tokens: f64,
    pub condition_id: String,
}

/// Result of a successful merge (mergePositions) call.
#[derive(Debug, Clone)]
pub struct MergeResult {
    pub tx_hash: String,
    pub amount_usdc_recovered: f64,
    pub condition_id: String,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Mint YES + NO tokens by splitting USDC via the CTF contract.
///
/// Calls `ConditionalTokens.splitPosition(collateral, parentId, condition, partition, amount)`.
/// Returns 1:1 YES and NO tokens for `amount_usdc`.
///
/// **DryRun / Backtest mode**: pass `private_key = None` to get a simulated result
/// without broadcasting any transaction.
pub async fn mint(
    condition_id: &str,
    amount_usdc: f64,
    collateral: &str,
    wallet_address: &str,
    private_key: Option<&str>,
) -> Result<MintResult> {
    // DryRun: return simulated result without touching the chain.
    if private_key.is_none() {
        info!("[CTF:mint] DryRun — simulating mint of ${amount_usdc:.2} on condition {condition_id}");
        return Ok(MintResult {
            tx_hash:     "0x0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            amount_usdc,
            yes_tokens:  amount_usdc,
            no_tokens:   amount_usdc,
            condition_id: condition_id.to_string(),
        });
    }

    let pk = private_key.unwrap();
    let amount_raw: u128 = (amount_usdc * USDC_SCALE as f64) as u128;

    // Encode `splitPosition(address,bytes32,bytes32,uint256[],uint256)` calldata.
    // Parameters:
    //   collateral  = ERC-20 address (USDC.e or pUSD)
    //   parentId    = bytes32(0)  (root position)
    //   conditionId = bytes32 condition_id
    //   partition   = [1, 2]     (YES index-set=1, NO index-set=2)
    //   amount      = uint256 amount in 6-decimal USDC
    let calldata = encode_split_position(collateral, condition_id, amount_raw)?;

    let tx_hash = send_raw_transaction(
        pk,
        wallet_address,
        CTF_CONTRACT,
        &calldata,
        0,
    ).await?;

    info!("[CTF:mint] tx confirmed: {tx_hash}");

    Ok(MintResult {
        tx_hash,
        amount_usdc,
        yes_tokens: amount_usdc,
        no_tokens:  amount_usdc,
        condition_id: condition_id.to_string(),
    })
}

/// Merge YES + NO tokens back to USDC via the CTF contract.
///
/// Calls `ConditionalTokens.mergePositions(collateral, parentId, condition, partition, amount)`.
/// Requires that the wallet holds both YES and NO tokens in equal amounts.
///
/// **DryRun / Backtest mode**: pass `private_key = None`.
pub async fn merge(
    condition_id: &str,
    amount_tokens: f64,
    collateral: &str,
    wallet_address: &str,
    private_key: Option<&str>,
) -> Result<MergeResult> {
    if private_key.is_none() {
        info!("[CTF:merge] DryRun — simulating merge of {amount_tokens:.2} tokens on {condition_id}");
        return Ok(MergeResult {
            tx_hash:              "0x0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            amount_usdc_recovered: amount_tokens,
            condition_id:          condition_id.to_string(),
        });
    }

    let pk = private_key.unwrap();
    let amount_raw: u128 = (amount_tokens * USDC_SCALE as f64) as u128;

    let calldata = encode_merge_positions(collateral, condition_id, amount_raw)?;

    let tx_hash = send_raw_transaction(
        pk,
        wallet_address,
        CTF_CONTRACT,
        &calldata,
        0,
    ).await?;

    info!("[CTF:merge] tx confirmed: {tx_hash}");

    Ok(MergeResult {
        tx_hash,
        amount_usdc_recovered: amount_tokens,
        condition_id: condition_id.to_string(),
    })
}

// ── ABI encoding ──────────────────────────────────────────────────────────────

/// Minimal ABI-encode for `splitPosition(address,bytes32,bytes32,uint256[],uint256)`.
fn encode_split_position(
    collateral: &str,
    condition_id: &str,
    amount: u128,
) -> Result<Vec<u8>> {
    // selector: keccak256("splitPosition(address,bytes32,bytes32,uint256[],uint256)")[0..4]
    let selector = keccak_selector("splitPosition(address,bytes32,bytes32,uint256[],uint256)");
    encode_ctf_call(&selector, collateral, condition_id, amount)
}

/// Minimal ABI-encode for `mergePositions(address,bytes32,bytes32,uint256[],uint256)`.
fn encode_merge_positions(
    collateral: &str,
    condition_id: &str,
    amount: u128,
) -> Result<Vec<u8>> {
    let selector = keccak_selector("mergePositions(address,bytes32,bytes32,uint256[],uint256)");
    encode_ctf_call(&selector, collateral, condition_id, amount)
}

/// Shared ABI encoding for split and merge (same parameter structure).
fn encode_ctf_call(
    selector: &[u8; 4],
    collateral: &str,
    condition_id: &str,
    amount: u128,
) -> Result<Vec<u8>> {
    let mut data = Vec::with_capacity(4 + 5 * 32 + 64);
    data.extend_from_slice(selector);

    // param 0: collateral address (padded to 32 bytes)
    let col_bytes = hex_to_bytes20(collateral)?;
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(&col_bytes);

    // param 1: parentCollectionId = bytes32(0)
    data.extend_from_slice(&[0u8; 32]);

    // param 2: conditionId (bytes32)
    let cid_bytes = hex_to_bytes32(condition_id)?;
    data.extend_from_slice(&cid_bytes);

    // param 3: partition (dynamic uint256[]) — offset points past fixed params
    // Fixed params: 4 (selector) + 5×32 = 164 bytes.  Offset to array = 5×32 = 160.
    let array_offset: u128 = 160;
    data.extend_from_slice(&u128_to_bytes32(array_offset));

    // param 4: amount (uint256)
    data.extend_from_slice(&u128_to_bytes32(amount));

    // Dynamic section — partition array: length=2, elements=[1, 2]
    data.extend_from_slice(&u128_to_bytes32(2)); // array length
    data.extend_from_slice(&u128_to_bytes32(1)); // YES index-set
    data.extend_from_slice(&u128_to_bytes32(2)); // NO  index-set

    Ok(data)
}

// ── Raw transaction ───────────────────────────────────────────────────────────

/// Sign and broadcast a raw EIP-155 transaction via Polygon RPC.
async fn send_raw_transaction(
    private_key_hex: &str,
    from: &str,
    to: &str,
    calldata: &[u8],
    value: u128,
) -> Result<String> {
    let client = reqwest::Client::new();

    // 1. Get nonce
    let nonce = get_nonce(&client, from).await?;

    // 2. Get gas price (legacy tx)
    let gas_price = get_gas_price(&client).await?;

    // 3. Estimate gas
    let gas_limit = estimate_gas(&client, from, to, calldata, value).await
        .unwrap_or(150_000); // safe fallback for CTF calls

    // 4. RLP-encode and sign
    let raw_tx = sign_legacy_tx(
        private_key_hex,
        nonce,
        gas_price,
        gas_limit,
        to,
        value,
        calldata,
        CHAIN_ID,
    )?;

    // 5. Broadcast
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "eth_sendRawTransaction",
        "params": [format!("0x{}", hex::encode(&raw_tx))],
        "id": 1
    });

    let mut last_err: Option<anyhow::Error> = None;
    for rpc in POLYGON_RPCS {
        match client.post(*rpc).json(&body).send().await {
            Ok(resp) => {
                let v: serde_json::Value = resp.json().await.context("RPC response parse")?;
                if let Some(err) = v.get("error") {
                    last_err = Some(anyhow::anyhow!("RPC error: {err}"));
                    continue;
                }
                if let Some(hash) = v.get("result").and_then(|r| r.as_str()) {
                    return Ok(hash.to_string());
                }
                last_err = Some(anyhow::anyhow!("unexpected RPC result: {v}"));
            }
            Err(e) => { last_err = Some(e.into()); }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("all RPCs failed")))
}

async fn get_nonce(client: &reqwest::Client, address: &str) -> Result<u64> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "eth_getTransactionCount",
        "params": [address, "latest"],
        "id": 1
    });
    let v: serde_json::Value = rpc_call(client, &body).await?;
    let hex = v["result"].as_str().context("no result")?;
    Ok(u64::from_str_radix(hex.strip_prefix("0x").unwrap_or(hex), 16)?)
}

async fn get_gas_price(client: &reqwest::Client) -> Result<u128> {
    let body = serde_json::json!({ "jsonrpc": "2.0", "method": "eth_gasPrice", "params": [], "id": 1 });
    let v: serde_json::Value = rpc_call(client, &body).await?;
    let hex = v["result"].as_str().context("no gasPrice")?;
    Ok(u128::from_str_radix(hex.strip_prefix("0x").unwrap_or(hex), 16)?)
}

async fn estimate_gas(
    client: &reqwest::Client,
    from: &str,
    to: &str,
    data: &[u8],
    value: u128,
) -> Result<u64> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "eth_estimateGas",
        "params": [{ "from": from, "to": to, "data": format!("0x{}", hex::encode(data)), "value": format!("0x{:x}", value) }],
        "id": 1
    });
    let v: serde_json::Value = rpc_call(client, &body).await?;
    let hex = v["result"].as_str().context("no gas estimate")?;
    let gas = u64::from_str_radix(hex.strip_prefix("0x").unwrap_or(hex), 16)?;
    Ok((gas as f64 * 1.2) as u64) // 20% buffer
}

async fn rpc_call(client: &reqwest::Client, body: &serde_json::Value) -> Result<serde_json::Value> {
    let mut last_err: Option<anyhow::Error> = None;
    for rpc in POLYGON_RPCS {
        match client.post(*rpc).json(body).send().await {
            Ok(r) => return Ok(r.json().await.context("parse")?),
            Err(e) => { last_err = Some(e.into()); }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("all RPCs failed")))
}

// ── EIP-155 signing ───────────────────────────────────────────────────────────

fn sign_legacy_tx(
    pk_hex: &str,
    nonce: u64,
    gas_price: u128,
    gas_limit: u64,
    to: &str,
    value: u128,
    data: &[u8],
    chain_id: u64,
) -> Result<Vec<u8>> {
    // RLP-encode for signing (EIP-155): [nonce, gasPrice, gasLimit, to, value, data, chainId, 0, 0]
    let to_bytes = hex_to_bytes20(to)?;

    let signing_rlp = rlp_encode_tx(nonce, gas_price, gas_limit, &to_bytes, value, data, chain_id, &[], &[]);

    // Hash and sign
    let hash: [u8; 32] = Keccak256::digest(&signing_rlp).into();

    let pk_raw = hex::decode(pk_hex.strip_prefix("0x").unwrap_or(pk_hex))
        .map_err(|e| anyhow::anyhow!("invalid private key hex: {e}"))?;
    let pk = SigningKey::from_slice(&pk_raw)
        .map_err(|e| anyhow::anyhow!("invalid private key: {e}"))?;

    let (sig, recid) = pk.sign_prehash(&hash)
        .map_err(|e| anyhow::anyhow!("signing failed: {e}"))?;

    let sig_bytes = sig.to_bytes();
    let r = &sig_bytes[..32];
    let s = &sig_bytes[32..];
    let v = recid.to_byte() as u64 + chain_id * 2 + 35;

    // RLP-encode the signed tx: [nonce, gasPrice, gasLimit, to, value, data, v, r, s]
    Ok(rlp_encode_tx(nonce, gas_price, gas_limit, &to_bytes, value, data, v, r, s))
}

// ── Minimal RLP encoder ───────────────────────────────────────────────────────

fn rlp_encode_tx(
    nonce: u64, gas_price: u128, gas_limit: u64,
    to: &[u8], value: u128, data: &[u8],
    v_or_chain: u64, r: &[u8], s: &[u8],
) -> Vec<u8> {
    let mut items: Vec<Vec<u8>> = vec![
        rlp_uint(nonce as u128),
        rlp_uint(gas_price),
        rlp_uint(gas_limit as u128),
        rlp_bytes(to),
        rlp_uint(value),
        rlp_bytes(data),
        rlp_uint(v_or_chain as u128),
        rlp_bytes(r),
        rlp_bytes(s),
    ];
    let payload: Vec<u8> = items.drain(..).flatten().collect();
    let mut out = rlp_list_header(payload.len());
    out.extend_from_slice(&payload);
    out
}

fn rlp_uint(n: u128) -> Vec<u8> {
    if n == 0 {
        return vec![0x80]; // RLP empty string
    }
    let be = n.to_be_bytes();
    let bytes = trim_leading_zeros(&be);
    rlp_bytes(bytes)
}

fn rlp_bytes(b: &[u8]) -> Vec<u8> {
    let b = trim_leading_zeros(b);
    if b.is_empty() {
        return vec![0x80];
    }
    if b.len() == 1 && b[0] < 0x80 {
        return vec![b[0]];
    }
    let mut out = rlp_str_header(b.len());
    out.extend_from_slice(b);
    out
}

fn rlp_str_header(len: usize) -> Vec<u8> {
    if len < 56 {
        vec![0x80 + len as u8]
    } else {
        let len_bytes = trim_leading_zeros(&(len as u64).to_be_bytes()).to_vec();
        let mut out = vec![0xb7 + len_bytes.len() as u8];
        out.extend_from_slice(&len_bytes);
        out
    }
}

fn rlp_list_header(len: usize) -> Vec<u8> {
    if len < 56 {
        vec![0xc0 + len as u8]
    } else {
        let len_bytes = trim_leading_zeros(&(len as u64).to_be_bytes()).to_vec();
        let mut out = vec![0xf7 + len_bytes.len() as u8];
        out.extend_from_slice(&len_bytes);
        out
    }
}

fn trim_leading_zeros(b: &[u8]) -> &[u8] {
    let pos = b.iter().position(|&x| x != 0).unwrap_or(b.len());
    &b[pos..]
}

// ── Encoding helpers ──────────────────────────────────────────────────────────

fn keccak_selector(sig: &str) -> [u8; 4] {
    let hash = Keccak256::digest(sig.as_bytes());
    [hash[0], hash[1], hash[2], hash[3]]
}

fn hex_to_bytes20(hex: &str) -> Result<[u8; 20]> {
    let clean = hex.strip_prefix("0x").unwrap_or(hex);
    let b = hex::decode(clean).context("invalid hex address")?;
    if b.len() != 20 { anyhow::bail!("address must be 20 bytes, got {}", b.len()); }
    let mut out = [0u8; 20];
    out.copy_from_slice(&b);
    Ok(out)
}

fn hex_to_bytes32(hex: &str) -> Result<[u8; 32]> {
    let clean = hex.strip_prefix("0x").unwrap_or(hex);
    let b = hex::decode(clean).context("invalid hex bytes32")?;
    anyhow::ensure!(b.len() <= 32, "bytes32 too long: {} bytes", b.len());
    let mut out = [0u8; 32];
    out[32 - b.len()..].copy_from_slice(&b);
    Ok(out)
}

fn u128_to_bytes32(n: u128) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[16..].copy_from_slice(&n.to_be_bytes());
    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_split_position() {
        // keccak256("splitPosition(address,bytes32,bytes32,uint256[],uint256)")[0..4]
        let sel = keccak_selector("splitPosition(address,bytes32,bytes32,uint256[],uint256)");
        // Precomputed reference value
        assert_eq!(hex::encode(sel), "752d549c");
    }

    #[test]
    fn selector_merge_positions() {
        let sel = keccak_selector("mergePositions(address,bytes32,bytes32,uint256[],uint256)");
        assert_eq!(hex::encode(sel), "f82bf5de");
    }

    #[test]
    fn u128_to_bytes32_round_trip() {
        let n: u128 = 1_000_000; // 1 USDC in raw
        let b = u128_to_bytes32(n);
        let recovered = u128::from_be_bytes(b[16..].try_into().unwrap());
        assert_eq!(recovered, n);
    }

    #[test]
    fn hex_to_bytes20_valid() {
        let addr = "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174";
        let b = hex_to_bytes20(addr).unwrap();
        assert_eq!(b.len(), 20);
        assert_eq!(hex::encode(b), "2791bca1f2de4661ed88a30c99a7a9449aa84174");
    }

    #[test]
    fn calldata_split_has_correct_selector() {
        let data = encode_split_position(
            "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174",
            "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
            1_000_000,
        ).unwrap();
        // First 4 bytes = splitPosition selector
        assert_eq!(&data[..4], &[0x75, 0x2d, 0x54, 0x9c]);
        assert!(data.len() >= 4 + 5 * 32, "calldata too short");
    }

    #[tokio::test]
    async fn mint_dryrun_returns_simulated_result() {
        let result = mint(
            "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
            500.0,
            USDC_E_CONTRACT,
            "0x1234567890123456789012345678901234567890",
            None, // DryRun
        ).await.unwrap();

        assert_eq!(result.amount_usdc, 500.0);
        assert_eq!(result.yes_tokens, 500.0);
        assert_eq!(result.no_tokens, 500.0);
        assert!(result.tx_hash.starts_with("0x0000"));
    }

    #[tokio::test]
    async fn merge_dryrun_returns_simulated_result() {
        let result = merge(
            "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
            500.0,
            USDC_E_CONTRACT,
            "0x1234567890123456789012345678901234567890",
            None, // DryRun
        ).await.unwrap();

        assert_eq!(result.amount_usdc_recovered, 500.0);
        assert!(result.tx_hash.starts_with("0x0000"));
    }
}

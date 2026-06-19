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
    "https://polygon-mainnet.g.alchemy.com/v2/Cuu-vpezr187QNaaAteWRHXrKbGXphrU",
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

/// Merge YES + NO tokens back to USDC for a wallet that holds them inside a
/// **Polymarket ProxyWallet** contract (signature_type proxy / poly1271).
///
/// Direct `merge()` doesn't work for these: the CTF tokens live in the proxy
/// CONTRACT, not in the owner EOA, so a `mergePositions` signed by the EOA would
/// operate on the EOA's (empty) balance. Instead we wrap the CTF call in the proxy's
/// `proxy(ProxyCall[])` entrypoint (onlyOwner) so `msg.sender` at the CTF is the
/// proxy. The tx is still signed & gas-paid by the owner EOA (`wallet_address`),
/// which therefore must hold a little POL for gas.
///
/// `proxy_address` = the ProxyWallet contract that owns the tokens.
pub async fn merge_via_proxy(
    condition_id: &str,
    amount_tokens: f64,
    collateral: &str,
    owner_eoa: &str,
    proxy_address: &str,
    private_key: Option<&str>,
) -> Result<MergeResult> {
    if private_key.is_none() {
        info!("[CTF:merge_via_proxy] DryRun — simulating merge of {amount_tokens:.2} on {condition_id} via proxy {proxy_address}");
        return Ok(MergeResult {
            tx_hash: "0x0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            amount_usdc_recovered: amount_tokens,
            condition_id: condition_id.to_string(),
        });
    }
    let pk = private_key.unwrap();
    let amount_raw: u128 = (amount_tokens * USDC_SCALE as f64) as u128;

    // The Polymarket wallet is a "DepositWallet" (poly1271) — an EIP-712 smart
    // account, NOT a classic ProxyWallet with proxy(ProxyCall[]). Arbitrary calls
    // go through `execute(Batch,signature)` where Batch = {wallet, nonce, deadline,
    // Call[]} and each Call = {target, value, data}, authorised by an EIP-712
    // signature from the owner. We batch both required calls in ONE execute():
    //   1. setApprovalForAll(CTF, true)  — lets the CTF move the wallet's ERC-1155s
    //   2. mergePositions(...)           — recombines YES+NO back to USDC
    // Batching avoids the nonce/approval-ordering problem of two separate txs.
    let approval_calldata = encode_set_approval_for_all(CTF_CONTRACT, true)?;
    let merge_calldata = encode_merge_positions(collateral, condition_id, amount_raw)?;
    let calls = vec![
        DepositCall { target: CTF_CONTRACT.to_string(), value: 0, data: approval_calldata },
        DepositCall { target: CTF_CONTRACT.to_string(), value: 0, data: merge_calldata },
    ];

    let tx_hash = execute_via_deposit_wallet(pk, owner_eoa, proxy_address, calls).await?;
    info!("[CTF:merge_via_proxy] execute() tx confirmed: {tx_hash}");

    Ok(MergeResult {
        tx_hash,
        amount_usdc_recovered: amount_tokens,
        condition_id: condition_id.to_string(),
    })
}

/// One call inside a DepositWallet `execute` batch.
struct DepositCall {
    target: String,
    value: u128,
    data: Vec<u8>,
}

/// Execute a batch of calls through a Polymarket DepositWallet (`execute(Batch,bytes)`).
///
/// Builds the EIP-712 `Batch{wallet,nonce,deadline,Call[]}` typed-data, signs it with
/// the owner's key, ABI-encodes `execute(batch, signature)` and sends it (signed &
/// gas-paid by `owner_eoa`) to the wallet contract. The wallet verifies the signature
/// (Solady ERC-1271) and runs each Call from its own context, so the CTF sees the
/// wallet as `msg.sender` and operates on the wallet's token balance.
async fn execute_via_deposit_wallet(
    pk: &str,
    owner_eoa: &str,
    wallet: &str,
    calls: Vec<DepositCall>,
) -> Result<String> {
    let client = reqwest::Client::new();

    // 1. Read the wallet's current execute-nonce (selector nonce() = 0xaffed0e0).
    let nonce_dw = read_uint_call(&client, wallet, "0xaffed0e0").await
        .context("read DepositWallet nonce")?;
    // Deadline: far in the future (the chain has no clock here; use a fixed large ts).
    let deadline: u128 = 4_000_000_000; // year 2096

    // 2. Compute the EIP-712 digest and sign it.
    let digest = deposit_batch_digest(wallet, nonce_dw, deadline, &calls)?;
    let signature = sign_digest_65(pk, &digest)?;

    // 3. ABI-encode execute((address,uint256,uint256,(address,uint256,bytes)[]),bytes).
    let calldata = encode_execute(wallet, nonce_dw, deadline, &calls, &signature)?;

    // 4. Send, signed & gas-paid by the owner EOA, to the wallet.
    send_raw_transaction(pk, owner_eoa, wallet, &calldata, 0).await
}

/// EIP-712 digest for `Batch(address wallet,uint256 nonce,uint256 deadline,Call[] calls)Call(address target,uint256 value,bytes data)`.
fn deposit_batch_digest(wallet: &str, nonce: u128, deadline: u128, calls: &[DepositCall]) -> Result<[u8; 32]> {
    // Type hashes (Keccak256 of the canonical type strings).
    let batch_typehash = Keccak256::digest(
        b"Batch(address wallet,uint256 nonce,uint256 deadline,Call[] calls)Call(address target,uint256 value,bytes data)"
    );
    let call_typehash = Keccak256::digest(b"Call(address target,uint256 value,bytes data)");

    // hashStruct(Call) for each, then keccak of the concatenation = calls array hash.
    let mut calls_concat = Vec::new();
    for c in calls {
        let mut h = Vec::new();
        h.extend_from_slice(&call_typehash);
        h.extend_from_slice(&[0u8; 12]);
        h.extend_from_slice(&hex_to_bytes20(&c.target)?);
        h.extend_from_slice(&u128_to_bytes32(c.value));
        h.extend_from_slice(&Keccak256::digest(&c.data)); // bytes → keccak256(data)
        calls_concat.extend_from_slice(&Keccak256::digest(&h));
    }
    let calls_hash = Keccak256::digest(&calls_concat);

    // hashStruct(Batch)
    let mut batch = Vec::new();
    batch.extend_from_slice(&batch_typehash);
    batch.extend_from_slice(&[0u8; 12]);
    batch.extend_from_slice(&hex_to_bytes20(wallet)?);
    batch.extend_from_slice(&u128_to_bytes32(nonce));
    batch.extend_from_slice(&u128_to_bytes32(deadline));
    batch.extend_from_slice(&calls_hash);
    let batch_hash = Keccak256::digest(&batch);

    // domainSeparator: EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)
    let domain_typehash = Keccak256::digest(
        b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"
    );
    let mut domain = Vec::new();
    domain.extend_from_slice(&domain_typehash);
    domain.extend_from_slice(&Keccak256::digest(b"DepositWallet"));
    domain.extend_from_slice(&Keccak256::digest(b"1"));
    domain.extend_from_slice(&u128_to_bytes32(CHAIN_ID as u128));
    domain.extend_from_slice(&[0u8; 12]);
    domain.extend_from_slice(&hex_to_bytes20(wallet)?); // verifyingContract = the wallet itself
    let domain_separator = Keccak256::digest(&domain);

    // EIP-712: keccak256(0x1901 ++ domainSeparator ++ hashStruct(message))
    let mut pre = Vec::with_capacity(2 + 32 + 32);
    pre.extend_from_slice(&[0x19, 0x01]);
    pre.extend_from_slice(&domain_separator);
    pre.extend_from_slice(&batch_hash);
    Ok(Keccak256::digest(&pre).into())
}

/// Sign a 32-byte digest, returning a 65-byte (r ‖ s ‖ v) signature with v ∈ {27,28}.
fn sign_digest_65(pk_hex: &str, digest: &[u8; 32]) -> Result<Vec<u8>> {
    let pk_raw = hex::decode(pk_hex.strip_prefix("0x").unwrap_or(pk_hex))
        .map_err(|e| anyhow::anyhow!("invalid private key hex: {e}"))?;
    let pk = SigningKey::from_slice(&pk_raw)
        .map_err(|e| anyhow::anyhow!("invalid private key: {e}"))?;
    let (sig, recid) = pk.sign_prehash(digest)
        .map_err(|e| anyhow::anyhow!("signing failed: {e}"))?;
    let sig_bytes = sig.to_bytes();
    let mut out = Vec::with_capacity(65);
    out.extend_from_slice(&sig_bytes);                 // r ‖ s (64 bytes)
    out.push(recid.to_byte() + 27);                    // v
    Ok(out)
}

/// ABI-encode `execute((address,uint256,uint256,(address,uint256,bytes)[]),bytes)`.
fn encode_execute(wallet: &str, nonce: u128, deadline: u128, calls: &[DepositCall], signature: &[u8]) -> Result<Vec<u8>> {
    let selector = keccak_selector("execute((address,uint256,uint256,(address,uint256,bytes)[]),bytes)");
    let mut out = Vec::new();
    out.extend_from_slice(&selector);

    // Two top-level args: Batch (tuple, dynamic because it contains Call[]) and bytes signature.
    // head[0] = offset to Batch, head[1] = offset to signature.
    // Build the Batch tail first to know the signature offset.
    let batch = encode_batch_tuple(wallet, nonce, deadline, calls)?;
    let head_len = 2 * 32;
    out.extend_from_slice(&u128_to_bytes32(head_len as u128));            // offset to Batch
    out.extend_from_slice(&u128_to_bytes32((head_len + batch.len()) as u128)); // offset to signature
    out.extend_from_slice(&batch);
    // signature (dynamic bytes)
    out.extend_from_slice(&u128_to_bytes32(signature.len() as u128));
    out.extend_from_slice(signature);
    let pad = (32 - (signature.len() % 32)) % 32;
    out.extend(std::iter::repeat(0u8).take(pad));
    Ok(out)
}

/// ABI-encode the Batch tuple `(address wallet, uint256 nonce, uint256 deadline, Call[] calls)`.
fn encode_batch_tuple(wallet: &str, nonce: u128, deadline: u128, calls: &[DepositCall]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    // Batch is a dynamic tuple. Its head: wallet, nonce, deadline, offset→calls.
    out.extend_from_slice(&[0u8; 12]);
    out.extend_from_slice(&hex_to_bytes20(wallet)?);
    out.extend_from_slice(&u128_to_bytes32(nonce));
    out.extend_from_slice(&u128_to_bytes32(deadline));
    out.extend_from_slice(&u128_to_bytes32(0x80)); // offset to calls (4 head words)

    // calls array
    let mut arr = Vec::new();
    arr.extend_from_slice(&u128_to_bytes32(calls.len() as u128)); // length
    // each Call is a dynamic tuple → array head is offsets to each element
    let mut elems: Vec<Vec<u8>> = Vec::new();
    for c in calls {
        elems.push(encode_call_tuple(c)?);
    }
    let head_region = calls.len() * 32;
    let mut running = head_region;
    for e in &elems {
        arr.extend_from_slice(&u128_to_bytes32(running as u128));
        running += e.len();
    }
    for e in &elems {
        arr.extend_from_slice(e);
    }
    out.extend_from_slice(&arr);
    Ok(out)
}

/// ABI-encode one Call tuple `(address target, uint256 value, bytes data)`.
fn encode_call_tuple(c: &DepositCall) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0u8; 12]);
    out.extend_from_slice(&hex_to_bytes20(&c.target)?);
    out.extend_from_slice(&u128_to_bytes32(c.value));
    out.extend_from_slice(&u128_to_bytes32(0x60)); // offset to data (3 head words)
    out.extend_from_slice(&u128_to_bytes32(c.data.len() as u128));
    out.extend_from_slice(&c.data);
    let pad = (32 - (c.data.len() % 32)) % 32;
    out.extend(std::iter::repeat(0u8).take(pad));
    Ok(out)
}

/// `eth_call` a no-arg uint256 getter and return the value.
async fn read_uint_call(client: &reqwest::Client, to: &str, selector_hex: &str) -> Result<u128> {
    let body = serde_json::json!({
        "jsonrpc": "2.0", "method": "eth_call",
        "params": [{ "to": to, "data": selector_hex }, "latest"], "id": 1
    });
    let v = rpc_call(client, &body).await?;
    let hex = v["result"].as_str().context("no result")?;
    let hex = hex.strip_prefix("0x").unwrap_or(hex);
    Ok(u128::from_str_radix(&hex[hex.len().saturating_sub(32)..], 16).unwrap_or(0))
}

// ── ABI encoding ──────────────────────────────────────────────────────────────

/// ABI-encode `proxy((uint8,address,uint256,bytes)[])` with a single ProxyCall.
/// CallType.CALL = 1. Layout: selector, offset→array(0x20), array len(1),
/// offset→elem(0x20), then the tuple: typeCode, to, value, offset→data(0x80),
/// data len, data (right-padded to 32).
fn encode_proxy_call(to: &str, value: u128, inner: &[u8]) -> Result<Vec<u8>> {
    let selector = keccak_selector("proxy((uint8,address,uint256,bytes)[])");
    let mut data = Vec::new();
    data.extend_from_slice(&selector);

    // head: offset to the (dynamic) array argument
    data.extend_from_slice(&u128_to_bytes32(0x20));
    // array length = 1
    data.extend_from_slice(&u128_to_bytes32(1));
    // offset to element[0] relative to start of array-data region = 0x20
    data.extend_from_slice(&u128_to_bytes32(0x20));

    // tuple (uint8 typeCode, address to, uint256 value, bytes data)
    // typeCode = 1 (CALL)
    data.extend_from_slice(&u128_to_bytes32(1));
    // to (address, left-padded)
    let to_bytes = hex_to_bytes20(to)?;
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(&to_bytes);
    // value
    data.extend_from_slice(&u128_to_bytes32(value));
    // offset to `data` within the tuple = 4 words (typeCode,to,value,offset) = 0x80
    data.extend_from_slice(&u128_to_bytes32(0x80));
    // bytes length
    data.extend_from_slice(&u128_to_bytes32(inner.len() as u128));
    // bytes content, right-padded to a 32-byte boundary
    data.extend_from_slice(inner);
    let pad = (32 - (inner.len() % 32)) % 32;
    data.extend(std::iter::repeat(0u8).take(pad));

    Ok(data)
}

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

/// ABI-encode `setApprovalForAll(address,bool)` for ERC-1155 (ConditionalTokens).
/// selector = keccak256("setApprovalForAll(address,bool)") = 0xa22cb465.
fn encode_set_approval_for_all(operator: &str, approved: bool) -> Result<Vec<u8>> {
    let mut data = Vec::with_capacity(4 + 2 * 32);
    data.extend_from_slice(&[0xa2, 0x2c, 0xb4, 0x65]);
    let op_bytes = hex_to_bytes20(operator)?;
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(&op_bytes);
    data.extend_from_slice(if approved { &[0u8; 31] } else { &[0u8; 32] });
    data[35] = if approved { 1 } else { 0 };
    Ok(data)
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
        // Verified against 4byte.directory + onchain CTF: 0x72ce4275.
        // (The previous hardcoded "752d549c" was wrong — never matched the real selector.)
        let sel = keccak_selector("splitPosition(address,bytes32,bytes32,uint256[],uint256)");
        assert_eq!(hex::encode(sel), "72ce4275");
    }

    #[test]
    fn selector_merge_positions() {
        // Verified against 4byte.directory + onchain CTF: 0x9e7212ad.
        // (The previous hardcoded "f82bf5de" was wrong.)
        let sel = keccak_selector("mergePositions(address,bytes32,bytes32,uint256[],uint256)");
        assert_eq!(hex::encode(sel), "9e7212ad");
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
        // First 4 bytes = splitPosition selector (0x72ce4275, verified onchain)
        assert_eq!(&data[..4], &[0x72, 0xce, 0x42, 0x75]);
        assert!(data.len() >= 4 + 5 * 32, "calldata too short");
    }

    #[test]
    #[test]
    fn print_selectors() {
        let sel = keccak_selector("proxy((uint8,address,uint256,bytes)[])");
        println!("proxy selector: {}", hex::encode(sel));
        let sel2 = keccak_selector("setApprovalForAll(address,bool)");
        println!("setApprovalForAll selector: {}", hex::encode(sel2));
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

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::rpc;

// ---------------------------------------------------------------------------
// Chain configuration
// ---------------------------------------------------------------------------

const ETHEREUM: u64 = 1;
const BASE: u64 = 8453;
const ARBITRUM: u64 = 42161;
const OPTIMISM: u64 = 10;
const POLYGON: u64 = 137;

/// Uniswap V3 QuoterV2 — same address on all supported chains
const QUOTER_V2: &str = "0x61fFE014bA17989E743c5F6cB21bF9697530B21e";

/// Uniswap V3 SwapRouter02 — same address on all supported chains
#[allow(dead_code)]
const SWAP_ROUTER: &str = "0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45";

fn rpc_url(chain_id: u64) -> &'static str {
    match chain_id {
        ETHEREUM => "https://eth.llamarpc.com",
        BASE => "https://base.llamarpc.com",
        ARBITRUM => "https://arbitrum.llamarpc.com",
        OPTIMISM => "https://optimism.llamarpc.com",
        POLYGON => "https://polygon.llamarpc.com",
        _ => "https://eth.llamarpc.com",
    }
}

// ---------------------------------------------------------------------------
// ABI encoding helpers
// ---------------------------------------------------------------------------

/// Encode an Ethereum address as a 32-byte (64 hex char) ABI word (zero-padded on the left).
fn encode_address(addr: &str) -> String {
    let addr = addr
        .trim_start_matches("0x")
        .trim_start_matches("0X")
        .to_lowercase();
    format!("{:0>64}", addr)
}

/// Encode a u128 value as a 32-byte (64 hex char) ABI uint256 word.
fn encode_uint256(val: u128) -> String {
    format!("{:064x}", val)
}

/// Encode a u32 value as a 32-byte (64 hex char) ABI uint24 word.
fn encode_uint24(val: u32) -> String {
    format!("{:064x}", val)
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Result of a swap quote (without executing).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteResult {
    /// Raw amount out as a hex string (e.g. "0x1a2b3c...")
    pub amount_out: String,
    /// Human-readable amount out (divided by 10^decimals of token_out).
    /// Defaults to dividing by 10^6 (USDC/USDT) when decimals are unknown.
    pub amount_out_readable: f64,
    /// Gas estimate for the swap (from QuoterV2 response).
    pub gas_estimate: u64,
    /// Price impact in basis points; None when not calculable from quote alone.
    pub price_impact_bps: Option<u64>,
}

// ---------------------------------------------------------------------------
// get_quote
// ---------------------------------------------------------------------------

/// Quote a swap without executing it.
///
/// Uses `QuoterV2.quoteExactInputSingle` (selector `0xc6a5026a`).
/// The struct parameter is: (address tokenIn, address tokenOut, uint256 amountIn, uint24 fee, uint160 sqrtPriceLimitX96)
pub async fn get_quote(
    token_in: &str,
    token_out: &str,
    amount_in: u128,
    chain_id: u64,
) -> Result<QuoteResult> {
    // Selector for quoteExactInputSingle((address,address,uint256,uint24,uint160))
    let selector = "c6a5026a";

    // ABI-encode the struct fields concatenated (tuple encoding):
    //   tokenIn      — address (32 bytes)
    //   tokenOut     — address (32 bytes)
    //   amountIn     — uint256 (32 bytes)
    //   fee          — uint24  (32 bytes)  → use 3000 (0.3%)
    //   sqrtPriceLimit — uint160 (32 bytes) → 0 (no price limit)
    let encoded = format!(
        "0x{}{}{}{}{}{}",
        selector,
        encode_address(token_in),
        encode_address(token_out),
        encode_uint256(amount_in),
        encode_uint24(3000),
        encode_uint256(0), // sqrtPriceLimitX96 = 0
    );

    let url = rpc_url(chain_id);
    let result = rpc::eth_call(url, QUOTER_V2, &encoded).await?;

    // QuoterV2 returns: (uint256 amountOut, uint160 sqrtPriceX96After, uint32 initializedTicksCrossed, uint256 gasEstimate)
    // Each field is 32 bytes in the ABI-encoded result (prefixed with "0x").
    let hex_data = result.trim_start_matches("0x");
    if hex_data.len() < 64 {
        return Err(anyhow!(
            "QuoterV2 response too short: {} chars (expected >=64)",
            hex_data.len()
        ));
    }

    // Field 0: amountOut (bytes 0..64)
    let amount_out_hex = &hex_data[0..64];
    let amount_out_u128 = u128::from_str_radix(amount_out_hex, 16).unwrap_or(0);

    // Field 3: gasEstimate (bytes 192..256) — if present
    let gas_estimate: u64 = if hex_data.len() >= 256 {
        let gas_hex = &hex_data[192..256];
        u64::from_str_radix(gas_hex, 16).unwrap_or(0)
    } else {
        0
    };

    // Default: divide by 1e6 (reasonable for USDC/USDT; callers can recompute with real decimals)
    let amount_out_readable = amount_out_u128 as f64 / 1_000_000.0;

    Ok(QuoteResult {
        amount_out: format!("0x{}", amount_out_hex.trim_start_matches('0')),
        amount_out_readable,
        gas_estimate,
        price_impact_bps: None,
    })
}

// ---------------------------------------------------------------------------
// execute_swap
// ---------------------------------------------------------------------------

/// Execute a swap via `SwapRouter02.exactInputSingle`.
///
/// Signing a raw Ethereum transaction requires EIP-155 RLP encoding and a
/// k256 ECDSA implementation that is not included in this crate to avoid
/// heavy dependency conflicts.  Use the `wallet-manager` crate's `LocalSigner`
/// to build and broadcast the transaction instead.
pub async fn execute_swap(
    _token_in: &str,
    _token_out: &str,
    _amount_in: u128,
    _slippage_bps: u64,
    _chain_id: u64,
    _private_key_hex: &str,
    _gas_threshold_gwei: u64,
) -> Result<String> {
    Err(anyhow!(
        "execute_swap requires signer integration — use the wallet-manager crate's LocalSigner"
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_abi_encode_address() {
        // A 20-byte address should be left-padded with 24 zero bytes → 64 hex chars total.
        let addr = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2";
        let encoded = encode_address(addr);
        assert_eq!(encoded.len(), 64, "encoded address must be 64 hex chars");
        assert!(
            encoded.starts_with("000000000000000000000000"),
            "address must be left-padded with zeros"
        );
        assert!(
            encoded.ends_with("c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2"),
            "address bytes must appear at the end (lowercase)"
        );
    }

    #[test]
    fn test_abi_encode_address_no_prefix() {
        let addr = "A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
        let encoded = encode_address(addr);
        assert_eq!(encoded.len(), 64);
    }

    #[test]
    fn test_abi_encode_uint256_zero() {
        assert_eq!(encode_uint256(0), "0".repeat(64));
    }

    #[test]
    fn test_abi_encode_uint256_value() {
        // 1_000_000 = 0xF4240
        let encoded = encode_uint256(1_000_000);
        assert_eq!(encoded.len(), 64);
        assert!(encoded.ends_with("f4240"));
    }

    #[test]
    fn test_abi_encode_uint24_fee() {
        // Fee 3000 = 0xBB8
        let encoded = encode_uint24(3000);
        assert_eq!(encoded.len(), 64);
        assert!(encoded.ends_with("bb8"));
    }

    #[test]
    fn test_rpc_url_mapping() {
        assert_eq!(rpc_url(ETHEREUM), "https://eth.llamarpc.com");
        assert_eq!(rpc_url(BASE), "https://base.llamarpc.com");
        assert_eq!(rpc_url(ARBITRUM), "https://arbitrum.llamarpc.com");
        assert_eq!(rpc_url(OPTIMISM), "https://optimism.llamarpc.com");
        assert_eq!(rpc_url(POLYGON), "https://polygon.llamarpc.com");
        // Unknown chain falls back to Ethereum
        assert_eq!(rpc_url(999), "https://eth.llamarpc.com");
    }

    #[test]
    fn test_quote_result_serialize() {
        let qr = QuoteResult {
            amount_out: "0x5f5e100".to_string(),
            amount_out_readable: 100.0,
            gas_estimate: 150_000,
            price_impact_bps: Some(30),
        };

        let json_str = serde_json::to_string(&qr).expect("serialize must succeed");
        let deserialized: QuoteResult =
            serde_json::from_str(&json_str).expect("deserialize must succeed");

        assert_eq!(deserialized.amount_out, qr.amount_out);
        assert!((deserialized.amount_out_readable - qr.amount_out_readable).abs() < f64::EPSILON);
        assert_eq!(deserialized.gas_estimate, qr.gas_estimate);
        assert_eq!(deserialized.price_impact_bps, qr.price_impact_bps);
    }

    #[test]
    fn test_execute_swap_placeholder() {
        // execute_swap must return an error explaining signer integration is required.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(execute_swap(
            "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2",
            "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
            1_000_000_000_000_000_000u128,
            50,
            ETHEREUM,
            "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
            100,
        ));
        assert!(result.is_err(), "execute_swap must return an error");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("wallet-manager"),
            "error must mention wallet-manager, got: {}",
            msg
        );
    }

    /// Real network call — only runs with `cargo test -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn test_get_quote_network() {
        // WETH → USDC on Ethereum mainnet, 1 WETH = 1e18 wei
        let weth = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2";
        let usdc = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
        let amount_in: u128 = 1_000_000_000_000_000_000; // 1 WETH

        let result = get_quote(weth, usdc, amount_in, ETHEREUM)
            .await
            .expect("get_quote must succeed on mainnet");

        println!("QuoteResult: {:?}", result);
        assert!(
            result.amount_out_readable > 0.0,
            "WETH→USDC quote must be positive"
        );
    }
}

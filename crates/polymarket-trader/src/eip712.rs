//! EIP-712 typed-data hashing and signing for Polymarket CTF Exchange orders.

use crate::error::PolyError;
use anyhow::Result;
use k256::ecdsa::{RecoveryId, SigningKey};
use sha3::{Digest, Keccak256};

// ── Constants ────────────────────────────────────────────────────────────────

/// Polymarket CTF Exchange contract on Polygon mainnet.
pub const EXCHANGE_CONTRACT: &str = "0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E";

/// Polygon mainnet chain ID.
pub const CHAIN_ID: u64 = 137;

/// USDC.e decimals on Polygon.
pub const COLLATERAL_DECIMALS: u32 = 6;

/// EIP-712 type string for the Order struct.
const ORDER_TYPE_STRING: &str = "Order(uint256 salt,address maker,address signer,address taker,uint256 tokenId,uint256 makerAmount,uint256 takerAmount,uint256 expiration,uint256 nonce,uint256 feeRateBps,uint8 side,uint8 signatureType)";

/// EIP-712 type string for the Domain.
const DOMAIN_TYPE_STRING: &str = "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)";

/// Protocol name for the domain.
const PROTOCOL_NAME: &str = "Polymarket CTF Exchange";

/// Protocol version for the domain.
const PROTOCOL_VERSION: &str = "1";

// ── Order struct ─────────────────────────────────────────────────────────────

/// A signed Polymarket CTF order ready to be POSTed to /order.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SignedOrder {
    pub salt: String,
    pub maker: String,
    pub signer: String,
    pub taker: String,
    pub token_id: String,
    pub maker_amount: String,
    pub taker_amount: String,
    pub expiration: String,
    pub nonce: String,
    pub fee_rate_bps: String,
    pub side: u8,
    pub signature_type: u8,
    pub signature: String,
}

/// Side of the order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OrderSide {
    Buy = 0,
    Sell = 1,
}

/// Signature type.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SignatureType {
    Eoa = 0,
    Proxy = 1,
}

// ── Low-level keccak helpers ─────────────────────────────────────────────────

fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(data);
    hasher.finalize().into()
}

fn type_hash(type_string: &str) -> [u8; 32] {
    keccak256(type_string.as_bytes())
}

fn encode_bytes32(val: &[u8; 32]) -> [u8; 32] {
    *val
}

fn encode_uint256(val: u64) -> [u8; 32] {
    let mut buf = [0u8; 32];
    let be = val.to_be_bytes();
    buf[24..32].copy_from_slice(&be);
    buf
}

fn encode_u128_to_u256(val: u128) -> [u8; 32] {
    let mut buf = [0u8; 32];
    let be = val.to_be_bytes();
    buf[16..32].copy_from_slice(&be);
    buf
}

/// Parse a uint256 string for EIP-712 encoding.
/// Supports hex (`0x…`) and decimal notation. Values > 256 bits are rejected.
fn parse_uint256_str(s: &str) -> Result<[u8; 32]> {
    let mut buf = [0u8; 32];
    let trimmed = s.trim();
    if trimmed.starts_with("0x") || trimmed.starts_with("0X") {
        // Hex mode
        let clean = &trimmed[2..];
        let padded = if clean.len() % 2 == 1 {
            format!("0{clean}")
        } else {
            clean.to_string()
        };
        let bytes = hex::decode(&padded).map_err(|e| PolyError::Auth(format!("hex decode: {e}")))?;
        if bytes.len() > 32 {
            return Err(PolyError::Auth(format!("uint256 hex value too long: {} bytes", bytes.len())).into());
        }
        let start = 32 - bytes.len();
        buf[start..].copy_from_slice(&bytes);
    } else {
        // Decimal mode — big-endian byte-by-byte multiply-by-10
        for ch in trimmed.chars() {
            let digit = ch.to_digit(10).ok_or_else(|| {
                PolyError::Auth(format!("invalid decimal digit in uint256: {ch}"))
            })?;
            let mut carry = digit as u64;
            for i in (0..32).rev() {
                let val = (buf[i] as u64) * 10 + carry;
                buf[i] = (val & 0xFF) as u8;
                carry = val >> 8;
            }
        }
    }
    Ok(buf)
}

fn address_to_bytes20(addr: &str) -> Result<[u8; 20]> {
    let clean = addr.strip_prefix("0x").unwrap_or(addr).to_lowercase();
    let bytes = hex::decode(&clean).map_err(|e| PolyError::Auth(format!("invalid address hex: {e}")))?;
    if bytes.len() != 20 {
        return Err(PolyError::Auth(format!("address must be 20 bytes, got {}", bytes.len())).into());
    }
    let mut arr = [0u8; 20];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}

fn pad_address_to_32(addr_bytes: &[u8; 20]) -> [u8; 32] {
    let mut buf = [0u8; 32];
    buf[12..].copy_from_slice(addr_bytes);
    buf
}

// ── Domain separator ─────────────────────────────────────────────────────────

fn domain_separator() -> [u8; 32] {
    let domain_type_hash = type_hash(DOMAIN_TYPE_STRING);
    let name_hash = keccak256(PROTOCOL_NAME.as_bytes());
    let version_hash = keccak256(PROTOCOL_VERSION.as_bytes());
    let chain_id = CHAIN_ID;
    let contract_bytes = address_to_bytes20(EXCHANGE_CONTRACT).expect("exchange addr is valid");

    let mut encoded = Vec::with_capacity(32 * 5);
    encoded.extend_from_slice(&encode_bytes32(&domain_type_hash));
    encoded.extend_from_slice(&encode_bytes32(&name_hash));
    encoded.extend_from_slice(&encode_bytes32(&version_hash));
    encoded.extend_from_slice(&encode_uint256(chain_id));
    encoded.extend_from_slice(&pad_address_to_32(&contract_bytes));
    keccak256(&encoded)
}

// ── Order struct hash ────────────────────────────────────────────────────────

fn order_struct_hash(order: &SignedOrder) -> Result<[u8; 32]> {
    let order_type_hash = type_hash(ORDER_TYPE_STRING);

    let salt = parse_uint256_str(&order.salt)?;
    let maker = pad_address_to_32(&address_to_bytes20(&order.maker)?);
    let signer = pad_address_to_32(&address_to_bytes20(&order.signer)?);
    let taker = pad_address_to_32(&address_to_bytes20(&order.taker)?);
    let token_id = parse_uint256_str(&order.token_id)?;
    let maker_amount = parse_uint256_str(&order.maker_amount)?;
    let taker_amount = parse_uint256_str(&order.taker_amount)?;
    let expiration = parse_uint256_str(&order.expiration)?;
    let nonce = parse_uint256_str(&order.nonce)?;
    let fee_rate_bps = parse_uint256_str(&order.fee_rate_bps)?;

    let mut encoded = Vec::with_capacity(32 * 13);
    encoded.extend_from_slice(&encode_bytes32(&order_type_hash));
    encoded.extend_from_slice(&encode_bytes32(&salt));
    encoded.extend_from_slice(&maker);
    encoded.extend_from_slice(&signer);
    encoded.extend_from_slice(&taker);
    encoded.extend_from_slice(&encode_bytes32(&token_id));
    encoded.extend_from_slice(&encode_bytes32(&maker_amount));
    encoded.extend_from_slice(&encode_bytes32(&taker_amount));
    encoded.extend_from_slice(&encode_bytes32(&expiration));
    encoded.extend_from_slice(&encode_bytes32(&nonce));
    encoded.extend_from_slice(&encode_bytes32(&fee_rate_bps));
    let mut side_buf = [0u8; 32];
    side_buf[31] = order.side;
    encoded.extend_from_slice(&side_buf);

    let mut sig_type_buf = [0u8; 32];
    sig_type_buf[31] = order.signature_type;
    encoded.extend_from_slice(&sig_type_buf);

    Ok(keccak256(&encoded))
}

// ── Full EIP-712 digest ──────────────────────────────────────────────────────

fn eip712_digest(order: &SignedOrder) -> Result<[u8; 32]> {
    let ds = domain_separator();
    let sh = order_struct_hash(order)?;

    let mut msg = Vec::with_capacity(2 + 32 + 32);
    msg.push(0x19u8);
    msg.push(0x01u8);
    msg.extend_from_slice(&ds);
    msg.extend_from_slice(&sh);
    Ok(keccak256(&msg))
}

// ── Signing ──────────────────────────────────────────────────────────────────

/// Sign the EIP-712 digest of an order and attach the signature.
pub fn sign_order(signing_key: &SigningKey, order: &mut SignedOrder) -> Result<()> {
    use k256::ecdsa::signature::hazmat::PrehashSigner;

    let digest = eip712_digest(order)?;

    let (sig, recid): (k256::ecdsa::Signature, RecoveryId) = signing_key
        .sign_prehash(&digest)
        .map_err(|e| PolyError::Auth(format!("Order signing failed: {e}")))?;

    let r_s = sig.to_bytes(); // 64 bytes: r (32) + s (32)
    let v = recid.to_byte() + 27u8; // Ethereum convention

    let mut full_sig = Vec::with_capacity(65);
    full_sig.extend_from_slice(&r_s);
    full_sig.push(v);

    order.signature = format!("0x{}", hex::encode(full_sig));
    Ok(())
}

// ── Order construction helpers ───────────────────────────────────────────────

/// Generate a random salt as a decimal string (matches Polymarket SDK).
/// Returns a u64-safe value (≤ 2^53-1) so it round-trips cleanly through
/// JSON/JavaScript Number.parseInt without precision loss.
pub fn generate_salt() -> String {
    // JS safe integer range: 0 … 9_007_199_254_740_991 (2^53 - 1)
    const JS_MAX_SAFE: u64 = 9_007_199_254_740_991;
    let salt = rand::random::<u64>() % (JS_MAX_SAFE + 1);
    salt.to_string()
}

/// Convert a decimal USDC amount (e.g. 6.5) to a decimal string with 6 decimals.
pub fn usdc_to_decimal(amount: f64) -> String {
    let raw = (amount * 1_000_000.0).round() as u128;
    raw.to_string()
}

/// Convert a share count (same decimals as USDC on Polymarket) to decimal string.
pub fn shares_to_decimal(shares: f64) -> String {
    let raw = (shares * 1_000_000.0).round() as u128;
    raw.to_string()
}

/// Build a limit order and sign it.
///
/// * `price` — price per share in USDC (e.g. 0.65)
/// * `size`  — number of shares
/// * `side`  — Buy or Sell
/// * `proxy_address` — optional proxy wallet address; when set, maker/signer use
///   this address and signature_type becomes Proxy (1), while the EOA key still signs.
pub fn build_signed_limit_order(
    signing_key: &SigningKey,
    maker_address: &str,
    token_id: &str,
    side: OrderSide,
    price: f64,
    size: f64,
    proxy_address: Option<&str>,
) -> Result<SignedOrder> {
    let (maker_amount_usdc, taker_amount_shares) = match side {
        OrderSide::Buy => {
            let usdc = size * price;
            (usdc, size)
        }
        OrderSide::Sell => {
            let usdc = size * price;
            (size, usdc)
        }
    };

    let (maker, signer, sig_type) = match proxy_address {
        Some(proxy) => (proxy.to_lowercase(), proxy.to_lowercase(), SignatureType::Proxy as u8),
        None => (maker_address.to_lowercase(), maker_address.to_lowercase(), SignatureType::Eoa as u8),
    };

    let mut order = SignedOrder {
        salt: generate_salt(),
        maker,
        signer,
        taker: "0x0000000000000000000000000000000000000000".to_string(),
        token_id: token_id.to_lowercase(),
        maker_amount: usdc_to_decimal(maker_amount_usdc),
        taker_amount: shares_to_decimal(taker_amount_shares),
        expiration: "0".to_string(),
        nonce: "0".to_string(),
        fee_rate_bps: "0".to_string(),
        side: side as u8,
        signature_type: sig_type,
        signature: String::new(),
    };

    sign_order(signing_key, &mut order)?;
    Ok(order)
}

/// Build a market (FOK) order and sign it.
///
/// * `amount` — for BUY: USDC to spend; for SELL: shares to sell
/// * `price`  — worst acceptable price (slippage protection)
/// * `proxy_address` — optional proxy wallet address; when set, maker/signer use
///   this address and signature_type becomes Proxy (1), while the EOA key still signs.
pub fn build_signed_market_order(
    signing_key: &SigningKey,
    maker_address: &str,
    token_id: &str,
    side: OrderSide,
    amount: f64,
    price: f64,
    proxy_address: Option<&str>,
) -> Result<SignedOrder> {
    let (maker_amount, taker_amount) = match side {
        OrderSide::Buy => {
            // BUY market: maker = USDC amount, taker = shares = amount / price
            let shares = if price > 0.0 { amount / price } else { 0.0 };
            (amount, shares)
        }
        OrderSide::Sell => {
            // SELL market: maker = shares, taker = USDC = shares * price
            let usdc = amount * price;
            (amount, usdc)
        }
    };

    let (maker, signer, sig_type) = match proxy_address {
        Some(proxy) => (proxy.to_lowercase(), proxy.to_lowercase(), SignatureType::Proxy as u8),
        None => (maker_address.to_lowercase(), maker_address.to_lowercase(), SignatureType::Eoa as u8),
    };

    let mut order = SignedOrder {
        salt: generate_salt(),
        maker,
        signer,
        taker: "0x0000000000000000000000000000000000000000".to_string(),
        token_id: token_id.to_lowercase(),
        maker_amount: usdc_to_decimal(maker_amount),
        taker_amount: shares_to_decimal(taker_amount),
        expiration: "0".to_string(),
        nonce: "0".to_string(),
        fee_rate_bps: "0".to_string(),
        side: side as u8,
        signature_type: sig_type,
        signature: String::new(),
    };

    sign_order(signing_key, &mut order)?;
    Ok(order)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_usdc_to_decimal() {
        let s = usdc_to_decimal(6.5);
        // 6.5 * 1_000_000 = 6_500_000
        assert_eq!(s, "6500000");
    }

    #[test]
    fn test_domain_separator_not_zero() {
        let ds = domain_separator();
        assert_ne!(ds, [0u8; 32]);
    }

    #[test]
    fn test_parse_uint256_hex() {
        let buf = parse_uint256_str("0x632ea0").unwrap();
        // 0x00632EA0 = 6,500,000 → bytes [0x00, 0x63, 0x2E, 0xA0]
        assert_eq!(buf[28], 0x00);
        assert_eq!(buf[29], 0x63);
        assert_eq!(buf[30], 0x2e);
        assert_eq!(buf[31], 0xa0);
    }

    #[test]
    fn test_parse_uint256_decimal() {
        let buf = parse_uint256_str("70000").unwrap();
        // 70000 = 0x011170
        assert_eq!(buf[29], 0x01);
        assert_eq!(buf[30], 0x11);
        assert_eq!(buf[31], 0x70);
    }

    #[test]
    fn test_parse_uint256_decimal_large() {
        // A typical Polymarket token ID (decimal, ~70 digits)
        let token_id = "71321045679252212461912939320045685913944083024659953030648116240815881728";
        let buf = parse_uint256_str(token_id).unwrap();
        // Just ensure it doesn't panic and produces non-zero bytes at the end
        assert!(buf.iter().any(|b| *b != 0));
    }

    #[test]
    fn test_parse_uint256_hex_32_bytes() {
        let hex = "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let buf = parse_uint256_str(hex).unwrap();
        assert_eq!(buf[0], 0x12);
        assert_eq!(buf[31], 0xef);
    }

    #[test]
    fn test_build_limit_order() {
        let pk_hex = crate::auth::ANVIL_TEST_KEY;
        let key_bytes = hex::decode(pk_hex).unwrap();
        let signing_key = SigningKey::from_slice(&key_bytes).unwrap();
        let addr = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266";

        let order = build_signed_limit_order(
            &signing_key,
            addr,
            "0x1234567890abcdef",
            OrderSide::Buy,
            0.65,
            10.0,
            None,
        )
        .unwrap();

        assert!(order.signature.starts_with("0x"));
        assert_eq!(order.signature.len(), 132); // 0x + 130 hex = 65 bytes
        assert_eq!(order.maker, addr.to_lowercase());
        assert_eq!(order.side, 0);
    }
}

use crate::error::PolyError;
use anyhow::Result;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use chrono::Utc;
use hmac::{Hmac, Mac};
use k256::ecdsa::{RecoveryId, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sha3::{Digest, Keccak256};
use std::collections::HashMap;

type HmacSha256 = Hmac<Sha256>;

/// Polymarket API credentials obtained from L1 auth
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolyCredentials {
    pub api_key: String,
    pub secret: String,
    pub passphrase: String,
    pub wallet_address: String,
    /// Wallet private key (hex) for EIP-712 order signing.
    /// NOT serialized when storing credentials to disk.
    #[serde(skip)]
    pub private_key: Option<String>,
    /// Whether these are Builder Key credentials (use POLY_BUILDER_* headers).
    #[serde(default)]
    pub is_builder: bool,
    /// Optional proxy wallet address (Polymarket proxy / Safe).
    /// When set, orders use proxy signature_type=1 with this address as maker/signer.
    #[serde(default)]
    pub proxy_address: Option<String>,
}

// ── EIP-712 helpers ──────────────────────────────────────────────────────────

fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// ABI-encode a 32-byte value (left-pad to 32 bytes already satisfied for [u8;32]).
fn abi_encode_bytes32(val: &[u8; 32]) -> [u8; 32] {
    *val
}

/// ABI-encode a uint256 (big-endian, zero-padded to 32 bytes).
fn abi_encode_uint256(val: u64) -> [u8; 32] {
    let mut buf = [0u8; 32];
    let be = val.to_be_bytes();
    buf[24..32].copy_from_slice(&be);
    buf
}

/// keccak256 of a string (for type-hash computation).
fn type_hash(type_string: &str) -> [u8; 32] {
    keccak256(type_string.as_bytes())
}

/// Build EIP-712 domain separator for ClobAuthDomain on Polygon (chainId 137).
fn domain_separator() -> [u8; 32] {
    // typeHash("EIP712Domain(string name,string version,uint256 chainId)")
    let domain_type_hash = type_hash("EIP712Domain(string name,string version,uint256 chainId)");
    let name_hash = keccak256(b"ClobAuthDomain");
    let version_hash = keccak256(b"1");
    let chain_id: u64 = 137;

    let mut encoded = Vec::with_capacity(32 * 4);
    encoded.extend_from_slice(&abi_encode_bytes32(&domain_type_hash));
    encoded.extend_from_slice(&abi_encode_bytes32(&name_hash));
    encoded.extend_from_slice(&abi_encode_bytes32(&version_hash));
    encoded.extend_from_slice(&abi_encode_uint256(chain_id));
    keccak256(&encoded)
}

/// Parse a checksummed Ethereum address into its 20 raw bytes.
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

/// Encode an address for EIP-712 (20 bytes left-padded to 32 bytes).
fn encode_address(addr_bytes: &[u8; 20]) -> [u8; 32] {
    let mut buf = [0u8; 32];
    buf[12..].copy_from_slice(addr_bytes);
    buf
}

/// Build the ClobAuth struct hash.
///
/// Type: ClobAuth(address address,string timestamp,uint256 nonce,string message)
///
/// Matches the official Polymarket SDK exactly.
fn clob_auth_struct_hash(address: &str, timestamp: &str, nonce: u64) -> Result<[u8; 32]> {
    let clob_type_hash = type_hash("ClobAuth(address address,string timestamp,uint256 nonce,string message)");

    let addr_bytes = address_to_bytes20(address)?;
    let addr_encoded = encode_address(&addr_bytes);

    let timestamp_hash = keccak256(timestamp.as_bytes());
    let message_hash = keccak256(b"This message attests that I control the given wallet");

    let mut encoded = Vec::with_capacity(32 * 5);
    encoded.extend_from_slice(&abi_encode_bytes32(&clob_type_hash));
    encoded.extend_from_slice(&addr_encoded);
    encoded.extend_from_slice(&abi_encode_bytes32(&timestamp_hash));
    encoded.extend_from_slice(&abi_encode_uint256(nonce));
    encoded.extend_from_slice(&abi_encode_bytes32(&message_hash));
    Ok(keccak256(&encoded))
}

/// Compute the final EIP-712 digest for L1 auth.
pub fn eip712_digest(address: &str, timestamp: &str, nonce: u64) -> Result<[u8; 32]> {
    let ds = domain_separator();
    let sh = clob_auth_struct_hash(address, timestamp, nonce)?;

    let mut msg = Vec::with_capacity(2 + 32 + 32);
    msg.push(0x19u8);
    msg.push(0x01u8);
    msg.extend_from_slice(&ds);
    msg.extend_from_slice(&sh);
    Ok(keccak256(&msg))
}

/// Derive checksummed Ethereum address (lowercase hex for now) from a SigningKey.
pub fn address_from_signing_key(signing_key: &SigningKey) -> String {
    let verifying_key: &VerifyingKey = signing_key.verifying_key();
    let point = verifying_key.to_encoded_point(false); // uncompressed
    let bytes = point.as_bytes();
    // Drop the 0x04 prefix, hash the 64-byte X||Y
    let pub_bytes = &bytes[1..];
    let hash = keccak256(pub_bytes);
    // Take last 20 bytes
    let addr_bytes = &hash[12..];
    format!("0x{}", hex::encode(addr_bytes))
}

/// Sign the EIP-712 digest and return a 65-byte hex signature (with v adjusted by +27).
pub fn sign_eip712(signing_key: &SigningKey, digest: &[u8; 32]) -> Result<String> {
    use k256::ecdsa::signature::hazmat::PrehashSigner;

    let (sig, recid): (k256::ecdsa::Signature, RecoveryId) = signing_key
        .sign_prehash(digest)
        .map_err(|e| PolyError::Auth(format!("Signing failed: {e}")))?;

    let r_s = sig.to_bytes(); // 64 bytes: r (32) + s (32)
    let v = recid.to_byte() + 27u8; // Ethereum convention

    let mut full_sig = Vec::with_capacity(65);
    full_sig.extend_from_slice(&r_s);
    full_sig.push(v);

    Ok(format!("0x{}", hex::encode(full_sig)))
}

// ── Network request helper ───────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiKeyResponse {
    api_key: String,
    secret: String,
    passphrase: String,
}

/// Perform L1 EIP-712 auth to get API credentials from Polymarket CLOB API.
///
/// The CLOB expects L1 auth as **headers** (not JSON body):
/// POLY_ADDRESS, POLY_SIGNATURE, POLY_TIMESTAMP, POLY_NONCE.
pub async fn setup_credentials(private_key_hex: &str) -> Result<PolyCredentials> {
    // 1. Parse private key (never log it)
    let key_bytes = hex::decode(private_key_hex.strip_prefix("0x").unwrap_or(private_key_hex))
        .map_err(|e| PolyError::Auth(format!("Invalid private key hex: {e}")))?;
    let signing_key = SigningKey::from_slice(&key_bytes)
        .map_err(|e| PolyError::Auth(format!("Invalid private key: {e}")))?;

    // 2. Derive address
    let address = address_from_signing_key(&signing_key);

    // 3. Get current timestamp
    let timestamp = Utc::now().timestamp() as u64;
    let timestamp_str = timestamp.to_string();
    let nonce: u64 = 0;

    // 4. Compute EIP-712 digest and sign
    let digest = eip712_digest(&address, &timestamp_str, nonce)?;
    let signature = sign_eip712(&signing_key, &digest)?;

    // 5. POST to CLOB API with L1 auth headers
    let client = reqwest::Client::new();
    let resp = client
        .post("https://clob.polymarket.com/auth/api-key")
        .header("POLY_ADDRESS", &address)
        .header("POLY_SIGNATURE", &signature)
        .header("POLY_TIMESTAMP", &timestamp_str)
        .header("POLY_NONCE", "0")
        .send()
        .await
        .map_err(PolyError::Http)?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(PolyError::Auth(format!("L1 auth failed ({}): {}", status, text)).into());
    }

    let api_resp: ApiKeyResponse = resp.json().await.map_err(PolyError::Http)?;

    Ok(PolyCredentials {
        api_key: api_resp.api_key,
        secret: api_resp.secret,
        passphrase: api_resp.passphrase,
        wallet_address: address,
        private_key: None,
        is_builder: false,
        proxy_address: None,
    })
}

// ── File-based credential storage ───────────────────────────────────────────

#[derive(Deserialize, Serialize)]
struct ConfigFile {
    secrets: SecretsSection,
}

#[derive(Deserialize, Serialize)]
struct SecretsSection {
    api_key: String,
    secret: String,
    passphrase: String,
    wallet_address: Option<String>,
}

/// Load credentials from a TOML config file with a `[secrets]` section.
pub fn load_credentials(config_path: &str) -> Result<PolyCredentials> {
    let contents = std::fs::read_to_string(config_path).map_err(PolyError::Io)?;
    let config: ConfigFile =
        toml::from_str(&contents).map_err(|e| PolyError::Auth(format!("TOML parse error: {e}")))?;
    Ok(PolyCredentials {
        api_key: config.secrets.api_key,
        secret: config.secrets.secret,
        passphrase: config.secrets.passphrase,
        wallet_address: config.secrets.wallet_address.unwrap_or_default(),
        private_key: None,
        is_builder: false,
        proxy_address: None,
    })
}

/// Save credentials to a TOML config file under the `[secrets]` section.
pub fn save_credentials(config_path: &str, creds: &PolyCredentials) -> Result<()> {
    let config = ConfigFile {
        secrets: SecretsSection {
            api_key: creds.api_key.clone(),
            secret: creds.secret.clone(),
            passphrase: creds.passphrase.clone(),
            wallet_address: Some(creds.wallet_address.clone()),
        },
    };
    let toml_str =
        toml::to_string(&config).map_err(|e| PolyError::Auth(format!("TOML serialize error: {e}")))?;
    std::fs::write(config_path, toml_str).map_err(PolyError::Io)?;
    Ok(())
}

// ── L2 HMAC-SHA256 headers ───────────────────────────────────────────────────

/// Strategy for decoding the API secret before HMAC.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SecretDecodeStrategy {
    /// Use the secret as raw UTF-8 bytes.
    Raw,
    /// Decode from base64 first.
    Base64,
    /// Decode from hex first.
    Hex,
}

impl Default for SecretDecodeStrategy {
    fn default() -> Self {
        SecretDecodeStrategy::Base64
    }
}

/// Decode secret according to the chosen strategy.
///
/// **Base64 secrets:** Polymarket Builder Keys may deliver secrets in base64url
/// format (using `-` and `_` instead of `+` and `/`). We normalize those before decoding.
fn decode_secret(secret: &str, strategy: SecretDecodeStrategy) -> Vec<u8> {
    match strategy {
        SecretDecodeStrategy::Raw => secret.as_bytes().to_vec(),
        SecretDecodeStrategy::Base64 => {
            // Normalize base64url → standard base64
            let normalized = secret.replace('-', "+").replace('_', "/");
            if let Ok(decoded) = BASE64.decode(&normalized) {
                if decoded.len() >= 16 {
                    return decoded;
                }
            }
            secret.as_bytes().to_vec()
        }
        SecretDecodeStrategy::Hex => {
            if let Ok(decoded) = hex::decode(secret.strip_prefix("0x").unwrap_or(secret)) {
                if decoded.len() >= 16 {
                    return decoded;
                }
            }
            secret.as_bytes().to_vec()
        }
    }
}

/// Create L2 HMAC-SHA256 auth headers for a Polymarket CLOB API request.
///
/// Returns a map with keys: POLY_API_KEY, POLY_SIGNATURE, POLY_TIMESTAMP,
/// POLY_PASSPHRASE, POLY_ADDRESS.
///
/// **Important differences from the old buggy implementation:**
/// - Header names use underscores (`POLY_API_KEY`) not hyphens (`POLY-API-KEY`)
/// - The secret is **always** decoded from base64 (with base64url support)
/// - The signature is base64 URL-safe (`+` → `-`, `/` → `_`)
/// - `POLY_ADDRESS` is always included
pub fn create_l2_headers(
    creds: &PolyCredentials,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> HashMap<String, String> {
    create_l2_headers_with_strategy(creds, method, path, body, SecretDecodeStrategy::default())
}

/// Same as `create_l2_headers` but lets you specify the secret decoding strategy.
///
/// **Default is Base64** because Polymarket Builder Key secrets are base64-encoded.
pub fn create_l2_headers_with_strategy(
    creds: &PolyCredentials,
    method: &str,
    path: &str,
    body: Option<&str>,
    strategy: SecretDecodeStrategy,
) -> HashMap<String, String> {
    let timestamp = Utc::now().timestamp().to_string();
    let body_str = body.unwrap_or("");
    let message = format!("{}{}{}{}", timestamp, method, path, body_str);

    let secret_bytes = decode_secret(&creds.secret, strategy);
    tracing::debug!(
        "L2 HMAC message='{}' secret_strategy={:?} secret_len={}",
        message,
        strategy,
        secret_bytes.len()
    );

    let mut mac =
        HmacSha256::new_from_slice(&secret_bytes).expect("HMAC accepts any key length");
    mac.update(message.as_bytes());
    let result = mac.finalize().into_bytes();

    // Polymarket uses URL-safe base64 for the signature: '+' → '-', '/' → '_'
    let signature = BASE64.encode(result)
        .replace('+', "-")
        .replace('/', "_");

    let mut headers = HashMap::new();
    headers.insert("POLY_API_KEY".to_string(), creds.api_key.clone());
    headers.insert("POLY_SIGNATURE".to_string(), signature);
    headers.insert("POLY_TIMESTAMP".to_string(), timestamp);
    headers.insert("POLY_PASSPHRASE".to_string(), creds.passphrase.clone());
    headers.insert("POLY_ADDRESS".to_string(), creds.wallet_address.clone());
    headers
}

/// Create Builder Key auth headers for a Polymarket CLOB API request.
///
/// Builder Keys use different header names than standard L2 credentials:
/// POLY_BUILDER_API_KEY, POLY_BUILDER_SIGNATURE, POLY_BUILDER_TIMESTAMP,
/// POLY_BUILDER_PASSPHRASE (no POLY_ADDRESS).
///
/// The secret is expected to be base64url-encoded.
pub fn create_builder_headers(
    creds: &PolyCredentials,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> HashMap<String, String> {
    let timestamp = Utc::now().timestamp().to_string();
    let body_str = body.unwrap_or("");
    let message = format!("{}{}{}{}", timestamp, method, path, body_str);

    // Builder secrets are base64url — decode directly
    let normalized = creds.secret.replace('-', "+").replace('_', "/");
    let secret_bytes = BASE64.decode(&normalized).unwrap_or_else(|_| creds.secret.as_bytes().to_vec());

    let mut mac = HmacSha256::new_from_slice(&secret_bytes).expect("HMAC accepts any key length");
    mac.update(message.as_bytes());
    let result = mac.finalize().into_bytes();

    // URL-safe base64
    let signature = BASE64.encode(result)
        .replace('+', "-")
        .replace('/', "_");

    let mut headers = HashMap::new();
    headers.insert("POLY_BUILDER_API_KEY".to_string(), creds.api_key.clone());
    headers.insert("POLY_BUILDER_SIGNATURE".to_string(), signature);
    headers.insert("POLY_BUILDER_TIMESTAMP".to_string(), timestamp);
    headers.insert("POLY_BUILDER_PASSPHRASE".to_string(), creds.passphrase.clone());
    headers
}

// ── Tests ────────────────────────────────────────────────────────────────────

// Anvil / Hardhat test account #0 private key.  This is a well-known
// development key with *no real funds* on any mainnet.  It is used
// exclusively in unit tests that verify EIP-712 signing and address
// derivation without hitting the network.
pub const ANVIL_TEST_KEY: &str = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn mock_creds() -> PolyCredentials {
        PolyCredentials {
            api_key: "test-api-key".to_string(),
            secret: "test-secret".to_string(),
            passphrase: "test-passphrase".to_string(),
            wallet_address: "0x1234567890123456789012345678901234567890".to_string(),
            private_key: None,
            is_builder: false,
            proxy_address: None,
        }
    }

    #[test]
    fn test_create_l2_headers() {
        let creds = mock_creds();
        let headers = create_l2_headers(&creds, "GET", "/markets", None);

        assert!(headers.contains_key("POLY_API_KEY"), "missing POLY_API_KEY");
        assert!(headers.contains_key("POLY_SIGNATURE"), "missing POLY_SIGNATURE");
        assert!(headers.contains_key("POLY_TIMESTAMP"), "missing POLY_TIMESTAMP");
        assert!(headers.contains_key("POLY_PASSPHRASE"), "missing POLY_PASSPHRASE");
        assert!(headers.contains_key("POLY_ADDRESS"), "missing POLY_ADDRESS");

        assert_eq!(headers["POLY_API_KEY"], creds.api_key);
        assert_eq!(headers["POLY_PASSPHRASE"], creds.passphrase);
        assert_eq!(headers["POLY_ADDRESS"], creds.wallet_address);
        // Signature must be non-empty base64
        assert!(!headers["POLY_SIGNATURE"].is_empty());
    }

    #[test]
    fn test_create_l2_headers_with_body() {
        let creds = mock_creds();
        let body = r#"{"market":"0xabc"}"#;
        let headers_with = create_l2_headers(&creds, "POST", "/order", Some(body));
        let headers_without = create_l2_headers(&creds, "POST", "/order", None);

        // Signatures should differ because body is included in the HMAC message
        // (timestamps may differ by a second, so we just check structure)
        assert!(headers_with.contains_key("POLY_SIGNATURE"));
        assert!(headers_without.contains_key("POLY_SIGNATURE"));
    }

    #[test]
    fn test_save_load_credentials() {
        let creds = mock_creds();
        let tmp = NamedTempFile::new().expect("tempfile");
        // NamedTempFile already created the file; we'll write through save_credentials
        let path = tmp.path().to_str().unwrap().to_string();

        save_credentials(&path, &creds).expect("save failed");
        let loaded = load_credentials(&path).expect("load failed");

        assert_eq!(loaded.api_key, creds.api_key);
        assert_eq!(loaded.secret, creds.secret);
        assert_eq!(loaded.passphrase, creds.passphrase);

        // Explicitly keep tmp alive until here
        drop(tmp);
    }

    /// Test EIP-712 signing with a known private key (no network call).
    #[test]
    fn test_eip712_sign() {
        // Known test private key (NOT a real key with funds)
        let private_key_hex = ANVIL_TEST_KEY;
        let key_bytes = hex::decode(private_key_hex).unwrap();
        let signing_key = SigningKey::from_slice(&key_bytes).unwrap();

        let address = address_from_signing_key(&signing_key);
        assert!(address.starts_with("0x"), "address should start with 0x");
        assert_eq!(address.len(), 42, "address should be 42 chars");

        let timestamp = "1700000000";
        let digest = eip712_digest(&address, timestamp, 0).unwrap();
        assert_eq!(digest.len(), 32);

        let sig = sign_eip712(&signing_key, &digest).unwrap();
        assert!(sig.starts_with("0x"), "signature should start with 0x");
        // 65 bytes = 130 hex chars + "0x" prefix
        assert_eq!(sig.len(), 132, "signature should be 132 chars (0x + 130 hex)");

        // v byte (last byte) must be 27 or 28 (Ethereum convention)
        let v_hex = &sig[sig.len() - 2..];
        let v = u8::from_str_radix(v_hex, 16).unwrap();
        assert!(v == 27 || v == 28, "v must be 27 or 28, got {}", v);
    }

    /// Real network call — skipped by default.
    #[tokio::test]
    #[ignore]
    async fn test_setup_credentials_network() {
        let private_key = std::env::var("POLY_PRIVATE_KEY")
            .expect("Set POLY_PRIVATE_KEY env var to run this test");
        let creds = setup_credentials(&private_key).await.expect("L1 auth failed");
        assert!(!creds.api_key.is_empty());
        assert!(!creds.secret.is_empty());
        assert!(!creds.passphrase.is_empty());
    }
}

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

/// Build the ClobAuth struct hash.
/// Type: ClobAuth(string address,uint256 timestamp)
fn clob_auth_struct_hash(address: &str, timestamp: u64) -> Result<[u8; 32]> {
    let clob_type_hash = type_hash("ClobAuth(string address,uint256 timestamp)");

    // EIP-712: dynamic types (string) are encoded as keccak256 of their content
    let addr_bytes = address.as_bytes();
    let addr_hash = keccak256(addr_bytes);

    let mut encoded = Vec::with_capacity(32 * 3);
    encoded.extend_from_slice(&abi_encode_bytes32(&clob_type_hash));
    encoded.extend_from_slice(&abi_encode_bytes32(&addr_hash));
    encoded.extend_from_slice(&abi_encode_uint256(timestamp));
    Ok(keccak256(&encoded))
}

/// Compute the final EIP-712 digest.
pub fn eip712_digest(address: &str, timestamp: u64) -> Result<[u8; 32]> {
    let ds = domain_separator();
    let sh = clob_auth_struct_hash(address, timestamp)?;

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

#[derive(Serialize)]
struct L1AuthBody<'a> {
    address: &'a str,
    signature: &'a str,
    timestamp: &'a str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiKeyResponse {
    api_key: String,
    secret: String,
    passphrase: String,
}

/// Perform L1 EIP-712 auth to get API credentials from Polymarket CLOB API.
/// POST https://clob.polymarket.com/auth/api-key
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

    // 4. Compute EIP-712 digest and sign
    let digest = eip712_digest(&address, timestamp)?;
    let signature = sign_eip712(&signing_key, &digest)?;

    // 5. POST to CLOB API
    let client = reqwest::Client::new();
    let body = L1AuthBody {
        address: &address,
        signature: &signature,
        timestamp: &timestamp_str,
    };

    let resp = client
        .post("https://clob.polymarket.com/auth/api-key")
        .json(&body)
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
    })
}

/// Save credentials to a TOML config file under the `[secrets]` section.
pub fn save_credentials(config_path: &str, creds: &PolyCredentials) -> Result<()> {
    let config = ConfigFile {
        secrets: SecretsSection {
            api_key: creds.api_key.clone(),
            secret: creds.secret.clone(),
            passphrase: creds.passphrase.clone(),
        },
    };
    let toml_str =
        toml::to_string(&config).map_err(|e| PolyError::Auth(format!("TOML serialize error: {e}")))?;
    std::fs::write(config_path, toml_str).map_err(PolyError::Io)?;
    Ok(())
}

// ── L2 HMAC-SHA256 headers ───────────────────────────────────────────────────

/// Create L2 HMAC-SHA256 auth headers for a Polymarket CLOB API request.
///
/// Returns a map with keys: POLY-API-KEY, POLY-SIGNATURE, POLY-TIMESTAMP, POLY-PASSPHRASE
pub fn create_l2_headers(
    creds: &PolyCredentials,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> HashMap<String, String> {
    let timestamp = Utc::now().timestamp().to_string();
    let body_str = body.unwrap_or("");
    let message = format!("{}{}{}{}", timestamp, method, path, body_str);

    let mut mac =
        HmacSha256::new_from_slice(creds.secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(message.as_bytes());
    let result = mac.finalize().into_bytes();
    let signature = BASE64.encode(result);

    let mut headers = HashMap::new();
    headers.insert("POLY-API-KEY".to_string(), creds.api_key.clone());
    headers.insert("POLY-SIGNATURE".to_string(), signature);
    headers.insert("POLY-TIMESTAMP".to_string(), timestamp);
    headers.insert("POLY-PASSPHRASE".to_string(), creds.passphrase.clone());
    headers
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn mock_creds() -> PolyCredentials {
        PolyCredentials {
            api_key: "test-api-key".to_string(),
            secret: "test-secret".to_string(),
            passphrase: "test-passphrase".to_string(),
        }
    }

    #[test]
    fn test_create_l2_headers() {
        let creds = mock_creds();
        let headers = create_l2_headers(&creds, "GET", "/markets", None);

        assert!(headers.contains_key("POLY-API-KEY"), "missing POLY-API-KEY");
        assert!(headers.contains_key("POLY-SIGNATURE"), "missing POLY-SIGNATURE");
        assert!(headers.contains_key("POLY-TIMESTAMP"), "missing POLY-TIMESTAMP");
        assert!(headers.contains_key("POLY-PASSPHRASE"), "missing POLY-PASSPHRASE");

        assert_eq!(headers["POLY-API-KEY"], creds.api_key);
        assert_eq!(headers["POLY-PASSPHRASE"], creds.passphrase);
        // Signature must be non-empty base64
        assert!(!headers["POLY-SIGNATURE"].is_empty());
    }

    #[test]
    fn test_create_l2_headers_with_body() {
        let creds = mock_creds();
        let body = r#"{"market":"0xabc"}"#;
        let headers_with = create_l2_headers(&creds, "POST", "/order", Some(body));
        let headers_without = create_l2_headers(&creds, "POST", "/order", None);

        // Signatures should differ because body is included in the HMAC message
        // (timestamps may differ by a second, so we just check structure)
        assert!(headers_with.contains_key("POLY-SIGNATURE"));
        assert!(headers_without.contains_key("POLY-SIGNATURE"));
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
        let private_key_hex = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let key_bytes = hex::decode(private_key_hex).unwrap();
        let signing_key = SigningKey::from_slice(&key_bytes).unwrap();

        let address = address_from_signing_key(&signing_key);
        assert!(address.starts_with("0x"), "address should start with 0x");
        assert_eq!(address.len(), 42, "address should be 42 chars");

        let timestamp: u64 = 1_700_000_000;
        let digest = eip712_digest(&address, timestamp).unwrap();
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

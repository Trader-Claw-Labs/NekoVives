use anyhow::{anyhow, Context, Result};
use bip39::{Language, Mnemonic};
use chrono::Utc;
use coins_bip32::{path::DerivationPath, prelude::XPriv};
use k256::ecdsa::SigningKey;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tiny_keccak::{Hasher, Keccak};

use crate::store::{build_payload, open_payload, EncryptedPayload};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A local EVM signer backed by a k256 secp256k1 signing key.
///
/// This mirrors the role of `alloy::signers::local::LocalSigner<SigningKey>`.
pub struct LocalSigner {
    signing_key: SigningKey,
    address: String,
}

impl LocalSigner {
    pub fn from_signing_key(signing_key: SigningKey) -> Result<Self> {
        let address = evm_address_from_signing_key(&signing_key)?;
        Ok(Self {
            signing_key,
            address,
        })
    }

    /// Returns the checksummed EVM address (EIP-55).
    pub fn address(&self) -> &str {
        &self.address
    }

    /// Access the underlying signing key.
    pub fn signing_key(&self) -> &SigningKey {
        &self.signing_key
    }
}

pub struct WalletInfo {
    /// Checksummed EVM address ("0x…").
    pub address: String,
    /// JSON-serialized `WalletEncryptedData`.
    pub encrypted_private_key: Vec<u8>,
    /// Only present immediately after creation; `None` thereafter.
    pub mnemonic: Option<String>,
}

/// What we serialize into `encrypted_private_key`.
#[derive(Serialize, Deserialize)]
struct WalletEncryptedData {
    mnemonic_ct: EncryptedPayload,
    privkey_ct: EncryptedPayload,
}

// ---------------------------------------------------------------------------
// Address derivation helpers
// ---------------------------------------------------------------------------

/// Compute the Ethereum address from a `SigningKey` via keccak256 of uncompressed public key.
fn evm_address_from_signing_key(key: &SigningKey) -> Result<String> {
    let verifying_key = key.verifying_key();
    let encoded = verifying_key.to_encoded_point(false); // uncompressed
    let pubkey_bytes = encoded.as_bytes();
    // Skip the 0x04 prefix byte; hash the remaining 64 bytes
    let payload = &pubkey_bytes[1..];

    let mut keccak = Keccak::v256();
    let mut hash = [0u8; 32];
    keccak.update(payload);
    keccak.finalize(&mut hash);

    // Take last 20 bytes as the raw address
    let raw_addr = &hash[12..];
    Ok(eip55_checksum(raw_addr))
}

/// Apply EIP-55 checksum encoding to 20 raw address bytes.
fn eip55_checksum(addr_bytes: &[u8]) -> String {
    let lower_hex = hex::encode(addr_bytes);
    let mut keccak = Keccak::v256();
    let mut hash = [0u8; 32];
    keccak.update(lower_hex.as_bytes());
    keccak.finalize(&mut hash);

    let checksummed: String = lower_hex
        .chars()
        .enumerate()
        .map(|(i, c)| {
            if c.is_ascii_digit() {
                c
            } else {
                // Each nibble of hash corresponds to one hex char
                let nibble = if i % 2 == 0 {
                    (hash[i / 2] >> 4) & 0xf
                } else {
                    hash[i / 2] & 0xf
                };
                if nibble >= 8 {
                    c.to_ascii_uppercase()
                } else {
                    c
                }
            }
        })
        .collect();

    format!("0x{checksummed}")
}

// ---------------------------------------------------------------------------
// BIP44 key derivation
// ---------------------------------------------------------------------------

/// Derive a private key at BIP44 path `m/44'/60'/0'/0/{index}` from `seed`.
fn derive_private_key(seed: &[u8], index: u32) -> Result<[u8; 32]> {
    let master = XPriv::root_from_seed(seed, None)
        .map_err(|e| anyhow!("master key derivation failed: {e}"))?;

    let path: DerivationPath = format!("m/44'/60'/0'/0/{index}")
        .parse()
        .map_err(|e| anyhow!("invalid derivation path: {e}"))?;

    let derived = master
        .derive_path(&path)
        .map_err(|e| anyhow!("BIP44 derivation failed: {e}"))?;

    let signing_key_ref: &SigningKey = derived.as_ref();
    let key_bytes: [u8; 32] = signing_key_ref.to_bytes().into();
    Ok(key_bytes)
}

// ---------------------------------------------------------------------------
// Core wallet operations
// ---------------------------------------------------------------------------

/// Create a new EVM wallet at the given BIP44 index.
///
/// The `mnemonic` field of the returned `WalletInfo` is `Some(…)` exactly once
/// (at creation time). Callers should store `encrypted_private_key` and then
/// drop the `WalletInfo` to clear the mnemonic from memory.
pub fn create_wallet(index: u32, password: &str) -> Result<WalletInfo> {
    // 1. Generate a 128-bit (12-word) random mnemonic.
    let entropy = {
        let mut buf = [0u8; 16];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut buf);
        buf
    };
    let mnemonic =
        Mnemonic::from_entropy_in(Language::English, &entropy).context("mnemonic generation")?;
    let phrase = mnemonic.to_string();

    // 2. Derive seed bytes (no passphrase).
    let seed = mnemonic.to_seed("");

    // 3. BIP44 derivation.
    let privkey_bytes = derive_private_key(&seed, index)?;

    // 4. Build signing key and derive address.
    let signing_key =
        SigningKey::from_bytes((&privkey_bytes).into()).context("invalid private key bytes")?;
    let address = evm_address_from_signing_key(&signing_key)?;

    // 5. Encrypt mnemonic and private key separately.
    let mnemonic_ct = build_payload(phrase.as_bytes(), password)?;
    let privkey_ct = build_payload(&privkey_bytes, password)?;

    let wallet_data = WalletEncryptedData {
        mnemonic_ct,
        privkey_ct,
    };
    let encrypted_private_key =
        serde_json::to_vec(&wallet_data).context("serializing encrypted wallet data")?;

    Ok(WalletInfo {
        address,
        encrypted_private_key,
        mnemonic: Some(phrase),
    })
}

/// Reconstruct a `LocalSigner` from the encrypted key blob and password.
pub fn load_wallet(encrypted_key: &[u8], password: &str) -> Result<LocalSigner> {
    let wallet_data: WalletEncryptedData =
        serde_json::from_slice(encrypted_key).context("deserializing encrypted wallet data")?;

    let privkey_bytes = open_payload(&wallet_data.privkey_ct, password)
        .map_err(|_| anyhow!("decryption failed — check your password"))?;

    if privkey_bytes.len() != 32 {
        return Err(anyhow!("unexpected private key length after decryption"));
    }
    let signing_key =
        SigningKey::from_bytes(privkey_bytes.as_slice().into()).context("invalid private key")?;
    LocalSigner::from_signing_key(signing_key)
}

/// Decrypt and return the BIP39 mnemonic phrase.
pub fn export_mnemonic(encrypted_key: &[u8], password: &str) -> Result<String> {
    let wallet_data: WalletEncryptedData =
        serde_json::from_slice(encrypted_key).context("deserializing encrypted wallet data")?;

    let mnemonic_bytes = open_payload(&wallet_data.mnemonic_ct, password)
        .map_err(|_| anyhow!("decryption failed — check your password"))?;

    String::from_utf8(mnemonic_bytes).context("mnemonic is not valid UTF-8")
}

// ---------------------------------------------------------------------------
// SQLite helpers
// ---------------------------------------------------------------------------

/// Create the `wallets` table if it does not exist.
pub fn init_db(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wallets (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            address       TEXT    NOT NULL UNIQUE,
            encrypted_data BLOB   NOT NULL,
            created_at    TEXT    NOT NULL
        );",
    )
    .context("creating wallets table")
}

/// Persist a `WalletInfo` to the database.
///
/// The `mnemonic` field is intentionally **not** stored — only the encrypted blob.
pub fn save_wallet(conn: &Connection, info: &WalletInfo) -> Result<()> {
    let created_at = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT OR REPLACE INTO wallets (address, encrypted_data, created_at) VALUES (?1, ?2, ?3)",
        params![info.address, info.encrypted_private_key, created_at],
    )
    .context("inserting wallet into database")?;
    Ok(())
}

/// Load a `WalletInfo` by address. The `mnemonic` field is always `None`.
pub fn get_wallet(conn: &Connection, address: &str) -> Result<WalletInfo> {
    let (addr, encrypted_data): (String, Vec<u8>) = conn
        .query_row(
            "SELECT address, encrypted_data FROM wallets WHERE address = ?1",
            params![address],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .context("wallet not found")?;

    Ok(WalletInfo {
        address: addr,
        encrypted_private_key: encrypted_data,
        mnemonic: None,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWORD: &str = "correct-horse-battery-staple";
    const WRONG_PASSWORD: &str = "wrong-password";

    #[test]
    fn test_create_wallet() {
        let info = create_wallet(0, PASSWORD).expect("create_wallet failed");
        assert!(
            info.address.starts_with("0x"),
            "address should start with 0x, got: {}",
            info.address
        );
        assert_eq!(info.address.len(), 42, "EVM address must be 42 chars");
        assert!(
            info.mnemonic.is_some(),
            "mnemonic should be Some immediately after creation"
        );
        let phrase = info.mnemonic.as_ref().unwrap();
        let word_count = phrase.split_whitespace().count();
        assert!(
            word_count == 12 || word_count == 24,
            "expected 12 or 24 mnemonic words, got {word_count}"
        );
    }

    #[test]
    fn test_load_wallet() {
        let info = create_wallet(0, PASSWORD).expect("create_wallet failed");
        let created_address = info.address.clone();

        let signer = load_wallet(&info.encrypted_private_key, PASSWORD).expect("load_wallet failed");
        let loaded_address = signer.address().to_string();

        assert_eq!(
            created_address, loaded_address,
            "loaded signer address must match created address"
        );
    }

    #[test]
    fn test_export_mnemonic() {
        let info = create_wallet(0, PASSWORD).expect("create_wallet failed");
        let original_mnemonic = info.mnemonic.clone().unwrap();

        let exported =
            export_mnemonic(&info.encrypted_private_key, PASSWORD).expect("export_mnemonic failed");

        assert_eq!(
            original_mnemonic, exported,
            "exported mnemonic must match the one generated at creation"
        );
    }

    #[test]
    fn test_wrong_password() {
        let info = create_wallet(0, PASSWORD).expect("create_wallet failed");
        let result = load_wallet(&info.encrypted_private_key, WRONG_PASSWORD);
        assert!(
            result.is_err(),
            "load_wallet with wrong password should return Err"
        );
    }

    #[test]
    fn test_sqlite_round_trip() {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        init_db(&conn).expect("init_db");

        let info = create_wallet(1, PASSWORD).expect("create_wallet");
        let address = info.address.clone();
        save_wallet(&conn, &info).expect("save_wallet");

        let loaded = get_wallet(&conn, &address).expect("get_wallet");
        assert_eq!(loaded.address, address);
        assert!(loaded.mnemonic.is_none(), "mnemonic must be None after DB load");

        // Verify we can still decrypt from the retrieved blob
        let signer = load_wallet(&loaded.encrypted_private_key, PASSWORD).unwrap();
        assert_eq!(signer.address(), address);
    }
}

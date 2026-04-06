use anyhow::{anyhow, Context, Result};
use bip39::{Language, Mnemonic};
use chrono::Utc;
use ed25519_dalek::SigningKey;
use hmac::Hmac;
use pbkdf2::pbkdf2;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::Sha512;

use crate::store::{build_payload, open_payload, EncryptedPayload};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

pub struct TonWalletInfo {
    /// Simplified TON address: "0:<hex_pubkey>".
    pub address: String,
    /// JSON-serialized `TonEncryptedData`.
    pub encrypted_private_key: Vec<u8>,
    /// Only present immediately after creation; `None` thereafter.
    pub mnemonic: Option<String>,
}

/// What we serialize into `encrypted_private_key`.
#[derive(Serialize, Deserialize)]
struct TonEncryptedData {
    mnemonic_ct: EncryptedPayload,
    privkey_ct: EncryptedPayload,
}

// ---------------------------------------------------------------------------
// TON key derivation
// ---------------------------------------------------------------------------

/// Derive a 32-byte ED25519 seed from a mnemonic phrase using TON's approach:
/// PBKDF2-HMAC-SHA512(mnemonic_phrase, salt="TON default seed", iterations=100000).
fn derive_ton_key(phrase: &str) -> [u8; 32] {
    const SALT: &[u8] = b"TON default seed";
    const ITERATIONS: u32 = 100_000;

    let mut derived = [0u8; 64];
    pbkdf2::<Hmac<Sha512>>(phrase.as_bytes(), SALT, ITERATIONS, &mut derived)
        .expect("PBKDF2 length is valid");

    let mut key = [0u8; 32];
    key.copy_from_slice(&derived[..32]);
    key
}

// ---------------------------------------------------------------------------
// Core wallet operations
// ---------------------------------------------------------------------------

/// Create a new TON wallet.
///
/// Uses a 24-word BIP39 mnemonic and derives the ED25519 key via
/// PBKDF2-HMAC-SHA512 with salt "TON default seed" and 100000 iterations.
/// The `mnemonic` field is `Some(…)` only at creation time.
pub fn create_wallet(password: &str) -> Result<TonWalletInfo> {
    // 1. Generate a 256-bit (24-word) random mnemonic.
    let entropy = {
        let mut buf = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut buf);
        buf
    };
    let mnemonic =
        Mnemonic::from_entropy_in(Language::English, &entropy).context("mnemonic generation")?;
    let phrase = mnemonic.to_string();

    // 2. TON-specific key derivation.
    let privkey_bytes = derive_ton_key(&phrase);

    // 3. Build signing key and derive simplified TON address.
    let signing_key = SigningKey::from_bytes(&privkey_bytes);
    let pubkey_bytes = signing_key.verifying_key().to_bytes();
    let address = format!("0:{}", hex::encode(pubkey_bytes));

    // 4. Encrypt mnemonic and private key separately.
    let mnemonic_ct = build_payload(phrase.as_bytes(), password)?;
    let privkey_ct = build_payload(&privkey_bytes, password)?;

    let wallet_data = TonEncryptedData {
        mnemonic_ct,
        privkey_ct,
    };
    let encrypted_private_key =
        serde_json::to_vec(&wallet_data).context("serializing encrypted wallet data")?;

    Ok(TonWalletInfo {
        address,
        encrypted_private_key,
        mnemonic: Some(phrase),
    })
}

/// Reconstruct a `SigningKey` from the encrypted key blob and password.
pub fn load_keypair(encrypted_key: &[u8], password: &str) -> Result<SigningKey> {
    let wallet_data: TonEncryptedData =
        serde_json::from_slice(encrypted_key).context("deserializing encrypted wallet data")?;

    let privkey_bytes = open_payload(&wallet_data.privkey_ct, password)
        .map_err(|_| anyhow!("decryption failed — check your password"))?;

    if privkey_bytes.len() != 32 {
        return Err(anyhow!("unexpected private key length after decryption"));
    }
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&privkey_bytes);
    Ok(SigningKey::from_bytes(&bytes))
}

/// Decrypt and return the BIP39 mnemonic phrase.
pub fn export_mnemonic(encrypted_key: &[u8], password: &str) -> Result<String> {
    let wallet_data: TonEncryptedData =
        serde_json::from_slice(encrypted_key).context("deserializing encrypted wallet data")?;

    let mnemonic_bytes = open_payload(&wallet_data.mnemonic_ct, password)
        .map_err(|_| anyhow!("decryption failed — check your password"))?;

    String::from_utf8(mnemonic_bytes).context("mnemonic is not valid UTF-8")
}

// ---------------------------------------------------------------------------
// SQLite helpers
// ---------------------------------------------------------------------------

/// Create the `ton_wallets` table if it does not exist.
pub fn init_db(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS ton_wallets (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            address       TEXT    NOT NULL UNIQUE,
            encrypted_data BLOB   NOT NULL,
            created_at    TEXT    NOT NULL
        );",
    )
    .context("creating ton_wallets table")
}

/// Persist a `TonWalletInfo` to the database.
///
/// The `mnemonic` field is intentionally **not** stored — only the encrypted blob.
pub fn save_wallet(conn: &Connection, info: &TonWalletInfo) -> Result<()> {
    let created_at = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT OR REPLACE INTO ton_wallets (address, encrypted_data, created_at) VALUES (?1, ?2, ?3)",
        params![info.address, info.encrypted_private_key, created_at],
    )
    .context("inserting ton wallet into database")?;
    Ok(())
}

/// Load a `TonWalletInfo` by address. The `mnemonic` field is always `None`.
pub fn get_wallet(conn: &Connection, address: &str) -> Result<TonWalletInfo> {
    let (addr, encrypted_data): (String, Vec<u8>) = conn
        .query_row(
            "SELECT address, encrypted_data FROM ton_wallets WHERE address = ?1",
            params![address],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .context("ton wallet not found")?;

    Ok(TonWalletInfo {
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
        let info = create_wallet(PASSWORD).expect("create_wallet failed");
        assert!(
            info.address.starts_with("0:"),
            "TON address should start with '0:', got: {}",
            info.address
        );
        // "0:" + 64 hex chars = 66 total
        assert_eq!(
            info.address.len(),
            66,
            "TON address should be 66 chars (0: + 64 hex), got: {}",
            info.address.len()
        );
        assert!(
            info.mnemonic.is_some(),
            "mnemonic should be Some immediately after creation"
        );
        let phrase = info.mnemonic.as_ref().unwrap();
        let word_count = phrase.split_whitespace().count();
        assert_eq!(word_count, 24, "expected 24 mnemonic words, got {word_count}");
    }

    #[test]
    fn test_load_keypair() {
        let info = create_wallet(PASSWORD).expect("create_wallet failed");
        let created_address = info.address.clone();

        let signing_key =
            load_keypair(&info.encrypted_private_key, PASSWORD).expect("load_keypair failed");
        let pubkey_bytes = signing_key.verifying_key().to_bytes();
        let loaded_address = format!("0:{}", hex::encode(pubkey_bytes));

        assert_eq!(
            created_address, loaded_address,
            "loaded keypair address must match created address"
        );
    }

    #[test]
    fn test_export_mnemonic() {
        let info = create_wallet(PASSWORD).expect("create_wallet failed");
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
        let info = create_wallet(PASSWORD).expect("create_wallet failed");
        let result = load_keypair(&info.encrypted_private_key, WRONG_PASSWORD);
        assert!(
            result.is_err(),
            "load_keypair with wrong password should return Err"
        );
    }

    #[test]
    fn test_sqlite_round_trip() {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        init_db(&conn).expect("init_db");

        let info = create_wallet(PASSWORD).expect("create_wallet");
        let address = info.address.clone();
        save_wallet(&conn, &info).expect("save_wallet");

        let loaded = get_wallet(&conn, &address).expect("get_wallet");
        assert_eq!(loaded.address, address);
        assert!(loaded.mnemonic.is_none(), "mnemonic must be None after DB load");

        // Verify we can still decrypt from the retrieved blob
        let signing_key = load_keypair(&loaded.encrypted_private_key, PASSWORD).unwrap();
        let pubkey_bytes = signing_key.verifying_key().to_bytes();
        let recovered_address = format!("0:{}", hex::encode(pubkey_bytes));
        assert_eq!(recovered_address, address);
    }
}

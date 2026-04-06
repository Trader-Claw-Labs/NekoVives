use anyhow::{anyhow, Context, Result};
use bip39::{Language, Mnemonic};
use chrono::Utc;
use ed25519_dalek::SigningKey;
use hmac::{Hmac, Mac};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::Sha512;

use crate::store::{build_payload, open_payload, EncryptedPayload};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

pub struct SolanaWalletInfo {
    /// Base58-encoded public key (Solana address).
    pub address: String,
    /// JSON-serialized `SolanaEncryptedData`.
    pub encrypted_private_key: Vec<u8>,
    /// Only present immediately after creation; `None` thereafter.
    pub mnemonic: Option<String>,
}

/// What we serialize into `encrypted_private_key`.
#[derive(Serialize, Deserialize)]
struct SolanaEncryptedData {
    mnemonic_ct: EncryptedPayload,
    privkey_ct: EncryptedPayload,
}

// ---------------------------------------------------------------------------
// SLIP-0010 key derivation for ED25519
// ---------------------------------------------------------------------------

type HmacSha512 = Hmac<Sha512>;

/// SLIP-0010 master key derivation from seed bytes.
///
/// Returns (private_key_bytes[32], chain_code[32]).
fn slip10_master_key(seed: &[u8]) -> ([u8; 32], [u8; 32]) {
    let mut mac =
        HmacSha512::new_from_slice(b"ed25519 seed").expect("HMAC can accept any key size");
    mac.update(seed);
    let result = mac.finalize().into_bytes();
    let mut key = [0u8; 32];
    let mut chain_code = [0u8; 32];
    key.copy_from_slice(&result[..32]);
    chain_code.copy_from_slice(&result[32..]);
    (key, chain_code)
}

/// SLIP-0010 hardened child key derivation.
///
/// `index` is the raw index value; the function adds the hardened offset (0x80000000).
/// Returns (child_private_key[32], child_chain_code[32]).
fn slip10_derive_child(
    parent_key: &[u8; 32],
    parent_chain_code: &[u8; 32],
    index: u32,
) -> ([u8; 32], [u8; 32]) {
    // Hardened child: data = 0x00 || parent_key || (index | 0x80000000)
    let hardened_index = index | 0x8000_0000u32;
    let mut data = Vec::with_capacity(1 + 32 + 4);
    data.push(0x00);
    data.extend_from_slice(parent_key);
    data.extend_from_slice(&hardened_index.to_be_bytes());

    let mut mac =
        HmacSha512::new_from_slice(parent_chain_code).expect("HMAC can accept any key size");
    mac.update(&data);
    let result = mac.finalize().into_bytes();

    let mut key = [0u8; 32];
    let mut chain_code = [0u8; 32];
    key.copy_from_slice(&result[..32]);
    chain_code.copy_from_slice(&result[32..]);
    (key, chain_code)
}

/// Derive a Solana private key via SLIP-0010 on path m/44'/501'/0'/0'.
///
/// All components are hardened as required by SLIP-0010 for ED25519.
fn derive_solana_key(seed: &[u8]) -> [u8; 32] {
    // Master key
    let (mut key, mut chain_code) = slip10_master_key(seed);
    // m/44'
    let (k, c) = slip10_derive_child(&key, &chain_code, 44);
    key = k;
    chain_code = c;
    // m/44'/501'
    let (k, c) = slip10_derive_child(&key, &chain_code, 501);
    key = k;
    chain_code = c;
    // m/44'/501'/0'
    let (k, c) = slip10_derive_child(&key, &chain_code, 0);
    key = k;
    chain_code = c;
    // m/44'/501'/0'/0'
    let (k, _) = slip10_derive_child(&key, &chain_code, 0);
    k
}

// ---------------------------------------------------------------------------
// Core wallet operations
// ---------------------------------------------------------------------------

/// Create a new Solana wallet.
///
/// Uses BIP44 path m/44'/501'/0'/0' (all hardened, SLIP-0010 ED25519).
/// The `mnemonic` field is `Some(…)` only at creation time.
pub fn create_wallet(_index: u32, password: &str) -> Result<SolanaWalletInfo> {
    // 1. Generate a 256-bit (24-word) random mnemonic.
    let entropy = {
        let mut buf = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut buf);
        buf
    };
    let mnemonic =
        Mnemonic::from_entropy_in(Language::English, &entropy).context("mnemonic generation")?;
    let phrase = mnemonic.to_string();

    // 2. Derive seed bytes (no passphrase).
    let seed = mnemonic.to_seed("");

    // 3. SLIP-0010 derivation for m/44'/501'/0'/0'.
    let privkey_bytes = derive_solana_key(&seed);

    // 4. Build signing key and derive address (base58 of pubkey).
    let signing_key = SigningKey::from_bytes(&privkey_bytes);
    let pubkey_bytes = signing_key.verifying_key().to_bytes();
    let address = bs58::encode(&pubkey_bytes).into_string();

    // 5. Encrypt mnemonic and private key separately.
    let mnemonic_ct = build_payload(phrase.as_bytes(), password)?;
    let privkey_ct = build_payload(&privkey_bytes, password)?;

    let wallet_data = SolanaEncryptedData {
        mnemonic_ct,
        privkey_ct,
    };
    let encrypted_private_key =
        serde_json::to_vec(&wallet_data).context("serializing encrypted wallet data")?;

    Ok(SolanaWalletInfo {
        address,
        encrypted_private_key,
        mnemonic: Some(phrase),
    })
}

/// Reconstruct a `SigningKey` from the encrypted key blob and password.
pub fn load_keypair(encrypted_key: &[u8], password: &str) -> Result<SigningKey> {
    let wallet_data: SolanaEncryptedData =
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
    let wallet_data: SolanaEncryptedData =
        serde_json::from_slice(encrypted_key).context("deserializing encrypted wallet data")?;

    let mnemonic_bytes = open_payload(&wallet_data.mnemonic_ct, password)
        .map_err(|_| anyhow!("decryption failed — check your password"))?;

    String::from_utf8(mnemonic_bytes).context("mnemonic is not valid UTF-8")
}

// ---------------------------------------------------------------------------
// SQLite helpers
// ---------------------------------------------------------------------------

/// Create the `solana_wallets` table if it does not exist.
pub fn init_db(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS solana_wallets (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            address       TEXT    NOT NULL UNIQUE,
            encrypted_data BLOB   NOT NULL,
            created_at    TEXT    NOT NULL
        );",
    )
    .context("creating solana_wallets table")
}

/// Persist a `SolanaWalletInfo` to the database.
///
/// The `mnemonic` field is intentionally **not** stored — only the encrypted blob.
pub fn save_wallet(conn: &Connection, info: &SolanaWalletInfo) -> Result<()> {
    let created_at = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT OR REPLACE INTO solana_wallets (address, encrypted_data, created_at) VALUES (?1, ?2, ?3)",
        params![info.address, info.encrypted_private_key, created_at],
    )
    .context("inserting solana wallet into database")?;
    Ok(())
}

/// Load a `SolanaWalletInfo` by address. The `mnemonic` field is always `None`.
pub fn get_wallet(conn: &Connection, address: &str) -> Result<SolanaWalletInfo> {
    let (addr, encrypted_data): (String, Vec<u8>) = conn
        .query_row(
            "SELECT address, encrypted_data FROM solana_wallets WHERE address = ?1",
            params![address],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .context("solana wallet not found")?;

    Ok(SolanaWalletInfo {
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
        // Solana base58 pubkey is 32 bytes = 44 chars in base58
        assert!(
            !info.address.is_empty(),
            "address should not be empty"
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
        let info = create_wallet(0, PASSWORD).expect("create_wallet failed");
        let created_address = info.address.clone();

        let signing_key =
            load_keypair(&info.encrypted_private_key, PASSWORD).expect("load_keypair failed");
        let pubkey_bytes = signing_key.verifying_key().to_bytes();
        let loaded_address = bs58::encode(&pubkey_bytes).into_string();

        assert_eq!(
            created_address, loaded_address,
            "loaded keypair address must match created address"
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

        let info = create_wallet(0, PASSWORD).expect("create_wallet");
        let address = info.address.clone();
        save_wallet(&conn, &info).expect("save_wallet");

        let loaded = get_wallet(&conn, &address).expect("get_wallet");
        assert_eq!(loaded.address, address);
        assert!(loaded.mnemonic.is_none(), "mnemonic must be None after DB load");

        // Verify we can still decrypt from the retrieved blob
        let signing_key = load_keypair(&loaded.encrypted_private_key, PASSWORD).unwrap();
        let recovered_address =
            bs58::encode(signing_key.verifying_key().to_bytes()).into_string();
        assert_eq!(recovered_address, address);
    }
}

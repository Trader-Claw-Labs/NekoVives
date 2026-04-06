use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use argon2::{Argon2, Params, Version};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("encryption failed")]
    EncryptionFailed,
    #[error("decryption failed — wrong password or corrupted data")]
    DecryptionFailed,
    #[error("key derivation failed: {0}")]
    KeyDerivation(String),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Encrypted payload: salt for KDF + ciphertext (nonce prepended).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EncryptedPayload {
    /// 32-byte random salt used for Argon2id key derivation, hex-encoded.
    pub salt: String,
    /// AES-256-GCM ciphertext with 12-byte nonce prepended, hex-encoded.
    pub ciphertext: String,
}

/// Derive a 32-byte key from `password` and `salt` using Argon2id.
pub fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; 32], StoreError> {
    let params = Params::new(65536, 3, 4, Some(32))
        .map_err(|e| StoreError::KeyDerivation(e.to_string()))?;
    let argon2 = Argon2::new(argon2::Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| StoreError::KeyDerivation(e.to_string()))?;
    Ok(key)
}

/// Encrypt `plaintext` with AES-256-GCM using the provided 32-byte key.
/// Returns a byte vector with the 12-byte nonce prepended to the ciphertext.
pub fn encrypt(plaintext: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, StoreError> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| StoreError::EncryptionFailed)?;
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let mut ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| StoreError::EncryptionFailed)?;
    // Prepend nonce to ciphertext
    let mut result = nonce_bytes.to_vec();
    result.append(&mut ciphertext);
    Ok(result)
}

/// Decrypt `ciphertext` (first 12 bytes = nonce) with AES-256-GCM using the provided key.
pub fn decrypt(ciphertext: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, StoreError> {
    if ciphertext.len() < 12 {
        return Err(StoreError::DecryptionFailed);
    }
    let (nonce_bytes, data) = ciphertext.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| StoreError::DecryptionFailed)?;
    let nonce = Nonce::from_slice(nonce_bytes);
    cipher
        .decrypt(nonce, data)
        .map_err(|_| StoreError::DecryptionFailed)
}

/// Build an `EncryptedPayload` by generating a fresh salt, deriving the key, and encrypting.
pub fn build_payload(plaintext: &[u8], password: &str) -> Result<EncryptedPayload, StoreError> {
    let mut salt = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut salt);
    let key = derive_key(password, &salt)?;
    let raw_ct = encrypt(plaintext, &key)?;
    Ok(EncryptedPayload {
        salt: hex::encode(salt),
        ciphertext: hex::encode(raw_ct),
    })
}

/// Decrypt an `EncryptedPayload` using the provided password.
pub fn open_payload(payload: &EncryptedPayload, password: &str) -> Result<Vec<u8>, StoreError> {
    let salt = hex::decode(&payload.salt).map_err(|_| StoreError::DecryptionFailed)?;
    let raw_ct = hex::decode(&payload.ciphertext).map_err(|_| StoreError::DecryptionFailed)?;
    let key = derive_key(password, &salt)?;
    decrypt(&raw_ct, &key)
}

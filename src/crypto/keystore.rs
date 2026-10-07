use super::secure_memory::ProtectedKey;
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use argon2::{
    password_hash::SaltString,
    Algorithm, Argon2, Params, Version,
};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize)]
pub struct KeystoreFile {
    pub version: u32,
    pub address: String,
    pub crypto: CryptoMeta,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CryptoMeta {
    pub cipher: String,
    pub ciphertext: String,
    pub nonce: String,
    pub kdf: String,
    pub kdfparams: KdfParams,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct KdfParams {
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
    pub salt: String,
}

/// Encrypts a private key using Argon2id + AES-256-GCM and persists it as a JSON keystore file.
pub fn encrypt_key_to_file(
    key: &ProtectedKey,
    address: &str,
    passphrase: &str,
    dest_path: &Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut salt_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut salt_bytes);
    let salt_hex = hex::encode(salt_bytes);

    // Argon2id parameters (industry-standard high security)
    let m_cost = 65536; // 64 MB
    let t_cost = 3;
    let p_cost = 4;

    let params = Params::new(m_cost, t_cost, p_cost, Some(32))
        .map_err(|e| format!("Argon2 params error: {e}"))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

    let mut derived_key = [0u8; 32];
    argon2
        .hash_password_into(passphrase.as_bytes(), &salt_bytes, &mut derived_key)
        .map_err(|e| format!("Key derivation error: {e}"))?;

    // Generate random 12-byte (96-bit) nonce for AES-256-GCM
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let cipher = Aes256Gcm::new_from_slice(&derived_key)
        .map_err(|e| format!("Cipher init error: {e}"))?;

    let ciphertext = cipher
        .encrypt(nonce, key.as_bytes().as_slice())
        .map_err(|e| format!("Encryption error: {e}"))?;

    // Zeroize derived key in RAM
    use zeroize::Zeroize;
    derived_key.zeroize();

    let keystore = KeystoreFile {
        version: 1,
        address: address.to_lowercase(),
        crypto: CryptoMeta {
            cipher: "aes-256-gcm".to_string(),
            ciphertext: hex::encode(ciphertext),
            nonce: hex::encode(nonce_bytes),
            kdf: "argon2id".to_string(),
            kdfparams: KdfParams {
                m_cost,
                t_cost,
                p_cost,
                salt: salt_hex,
            },
        },
    };

    let serialized = serde_json::to_string_pretty(&keystore)?;
    if let Some(parent) = dest_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(dest_path, serialized)?;

    Ok(())
}

/// Decrypts an Argon2id + AES-256-GCM keystore file into a memory-pinned `ProtectedKey`.
pub fn decrypt_key_from_file(
    file_path: &Path,
    passphrase: &str,
) -> Result<(ProtectedKey, String), Box<dyn std::error::Error + Send + Sync>> {
    let content = fs::read_to_string(file_path)?;
    let keystore: KeystoreFile = serde_json::from_str(&content)?;

    let salt_bytes = hex::decode(&keystore.crypto.kdfparams.salt)?;
    let nonce_bytes = hex::decode(&keystore.crypto.nonce)?;
    let ciphertext_bytes = hex::decode(&keystore.crypto.ciphertext)?;

    let params = Params::new(
        keystore.crypto.kdfparams.m_cost,
        keystore.crypto.kdfparams.t_cost,
        keystore.crypto.kdfparams.p_cost,
        Some(32),
    )
    .map_err(|e| format!("Argon2 params error: {e}"))?;

    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

    let mut derived_key = [0u8; 32];
    argon2
        .hash_password_into(passphrase.as_bytes(), &salt_bytes, &mut derived_key)
        .map_err(|e| format!("Key derivation error: {e}"))?;

    let cipher = Aes256Gcm::new_from_slice(&derived_key)
        .map_err(|e| format!("Cipher init error: {e}"))?;

    let nonce = Nonce::from_slice(&nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext_bytes.as_slice())
        .map_err(|_| "Failed to decrypt keystore: invalid passphrase or corrupt data")?;

    // Zeroize derived key in RAM
    use zeroize::Zeroize;
    derived_key.zeroize();

    if plaintext.len() != 32 {
        return Err("Decrypted key payload is not 32 bytes".into());
    }

    let mut key_bytes = [0u8; 32];
    key_bytes.copy_from_slice(&plaintext);

    let protected = ProtectedKey::new(key_bytes);
    Ok((protected, keystore.address))
}

use super::secure_memory::ProtectedKey;
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use argon2::{
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
    let nonce = Nonce::from(nonce_bytes);

    let cipher = Aes256Gcm::new_from_slice(&derived_key)
        .map_err(|e| format!("Cipher init error: {e}"))?;

    let ciphertext = cipher
        .encrypt(&nonce, key.as_bytes().as_slice())
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new().recursive(true).mode(0o700).create(parent)?;
        }
        #[cfg(not(unix))]
        {
            std::fs::create_dir_all(parent)?;
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(dest_path)?;
        std::io::Write::write_all(&mut file, serialized.as_bytes())?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(dest_path, serialized.as_bytes())?;
    }

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

    if nonce_bytes.len() != 12 {
        return Err(format!("Invalid nonce length: expected 12 bytes, got {}", nonce_bytes.len()).into());
    }

    if keystore.crypto.kdfparams.m_cost > 1_048_576 || keystore.crypto.kdfparams.t_cost > 10 {
        return Err("Keystore kdfparams exceed safe limits (potential DoS)".into());
    }

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

    let mut nonce_arr = [0u8; 12];
    nonce_arr.copy_from_slice(&nonce_bytes);
    let nonce = Nonce::from(nonce_arr);
    
    let mut plaintext = cipher
        .decrypt(&nonce, ciphertext_bytes.as_slice())
        .map_err(|_| "Failed to decrypt keystore: invalid passphrase or corrupt data")?;

    // Zeroize derived key in RAM
    use zeroize::Zeroize;
    derived_key.zeroize();

    if plaintext.len() != 32 {
        plaintext.zeroize();
        return Err("Decrypted key payload is not 32 bytes".into());
    }

    let mut protected = ProtectedKey::empty();
    protected.as_mut_bytes().copy_from_slice(&plaintext);
    plaintext.zeroize();

    Ok((protected, keystore.address))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keystore_rejects_corrupted_nonce() {
        let temp_dir = std::env::temp_dir().join("pulse_keystore_test_nonce");
        let _ = std::fs::create_dir_all(&temp_dir);
        let key_path = temp_dir.join("bad_nonce.json");

        let bad_json = r#"{
            "version": 1,
            "address": "0x1234567890123456789012345678901234567890",
            "crypto": {
                "cipher": "aes-256-gcm",
                "ciphertext": "0011223344",
                "nonce": "001122",
                "kdf": "argon2id",
                "kdfparams": {
                    "m_cost": 65536,
                    "t_cost": 3,
                    "p_cost": 4,
                    "salt": "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff"
                }
            }
        }"#;
        std::fs::write(&key_path, bad_json).unwrap();
        let res = decrypt_key_from_file(&key_path, "password");
        assert!(res.is_err());
        let _ = std::fs::remove_file(key_path);
    }

    #[test]
    fn test_keystore_rejects_dos_mcost() {
        let temp_dir = std::env::temp_dir().join("pulse_keystore_test_dos");
        let _ = std::fs::create_dir_all(&temp_dir);
        let key_path = temp_dir.join("dos_mcost.json");

        let bad_json = r#"{
            "version": 1,
            "address": "0x1234567890123456789012345678901234567890",
            "crypto": {
                "cipher": "aes-256-gcm",
                "ciphertext": "0011223344",
                "nonce": "00112233445566778899aabb",
                "kdf": "argon2id",
                "kdfparams": {
                    "m_cost": 2000000,
                    "t_cost": 3,
                    "p_cost": 4,
                    "salt": "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff"
                }
            }
        }"#;
        std::fs::write(&key_path, bad_json).unwrap();
        let res = decrypt_key_from_file(&key_path, "password");
        assert!(res.is_err());
        let _ = std::fs::remove_file(key_path);
    }
}

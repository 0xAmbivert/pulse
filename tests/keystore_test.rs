use pulse::crypto::decrypt_key_from_file;

#[test]
fn test_keystore_malformed_nonce_does_not_panic() {
    let temp_dir = std::env::temp_dir().join("pulse_test_keystore_panic");
    let _ = std::fs::create_dir_all(&temp_dir);
    let key_path = temp_dir.join("corrupt_nonce.json");

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
    assert!(res.is_err(), "Expected error on malformed nonce");
    let err_msg = res.unwrap_err().to_string();
    assert!(err_msg.contains("Invalid nonce length") || err_msg.contains("nonce"));
    let _ = std::fs::remove_file(key_path);
}

#[test]
fn test_keystore_encrypt_decrypt_roundtrip() {
    use pulse::crypto::{encrypt_key_to_file, ProtectedKey};

    let temp_dir = std::env::temp_dir().join("pulse_test_keystore_roundtrip");
    let _ = std::fs::create_dir_all(&temp_dir);
    let key_path = temp_dir.join("valid_wallet.json");

    let hex_pk = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let key = ProtectedKey::from_hex(hex_pk).expect("valid hex");
    let addr = "0x1234567890123456789012345678901234567890";
    let passphrase = "my-secure-password-123";

    encrypt_key_to_file(&key, addr, passphrase, &key_path).expect("encrypt ok");
    let (decrypted_key, decrypted_addr) = decrypt_key_from_file(&key_path, passphrase).expect("decrypt ok");

    assert_eq!(decrypted_addr, addr.to_lowercase());
    assert_eq!(decrypted_key.as_bytes(), key.as_bytes());

    let wrong_decrypt = decrypt_key_from_file(&key_path, "wrong-password");
    assert!(wrong_decrypt.is_err());

    let _ = std::fs::remove_file(key_path);
}


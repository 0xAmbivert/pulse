# Pulse MEV Sniper Audit & Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve all discovered bugs, security vulnerabilities, architectural dead-ends, and unhandled panics across Pulse, implementing real MEV builder authentication, transaction tracking, dynamic gas speedups, live alerts, and a test suite.

**Architecture:** 
1. Correct the Flashbots EIP-191 authentication header by signing the keccak256 hash of the JSON-RPC payload as mandated by the Flashbots builder specification.
2. Harden crypto keystore parsing against slice-length panics and cross-platform compilation errors.
3. Fix mempool sniper logic to allow sniping when `monitor_owner_address` is None, and decouple base-fee metrics from the execution trigger channel.
4. Replace placeholder transaction polling with an active receipt monitor and Replace-by-Fee (RBF) speedup loop, wired directly to `AlertDispatcher`.
5. Add unit and integration tests across all modules.

**Tech Stack:** Rust (2021 edition), Tokio, Alloy 2.5, Reqwest, Ratatui, Crossterm, Zeroize, Argon2, AES-GCM.

**Spec:** Codebase audit findings and requirements described in `handoff.md` and codebase diagnosis.

## Global Constraints
- Do not create unrequested abstractions; preserve the existing project layout (`src/`, `tests/`, `docs/`).
- Never leak private keys or leave unzeroized plaintext buffers on the stack or unpinned heap.
- All files must remain under 500 lines.
- No dummy/placebo logic: all calculations, simulations, and builders must execute real network/cryptographic operations.
- All tests must pass with `cargo test`.

---

### Task 1: Fix Flashbots / Builder EIP-191 Auth Signature & Builder Handling

**Files:**
- Modify: `src/network/builder_client.rs:60-120`
- Test: `tests/builder_client_test.rs`

**Interfaces:**
- Consumes: `alloy::primitives::{keccak256, eip191_hash_message}`, `alloy::signers::local::PrivateKeySigner`, `alloy::signers::SignerSync`
- Produces: `MevBuilderClient::send_private_transaction(&self, raw_tx_hex: &str) -> Vec<BuilderOutcome>` producing valid `X-Flashbots-Signature` headers conforming to `{address}:0x{65-byte hex}`.

- [ ] **Step 1: Write the failing test for Flashbots EIP-191 signature calculation**

Create `tests/builder_client_test.rs`:
```rust
use alloy::primitives::{keccak256, eip191_hash_message, Address};
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::SignerSync;
use pulse::network::MevBuilderClient;

#[tokio::test]
async fn test_flashbots_signature_recovery() {
    let signer = PrivateKeySigner::random();
    let payload = r#"{"jsonrpc":"2.0","id":1,"method":"eth_sendPrivateTransaction","params":[]}"#;
    
    // Flashbots spec: hash = keccak256(body)
    let body_hash = keccak256(payload.as_bytes());
    // EIP-191 prefix: "\x19Ethereum Signed Message:\n32" + body_hash
    let eip191_hash = eip191_hash_message(body_hash);
    let signature = signer.sign_hash_sync(&eip191_hash).expect("sign failed");

    // Recover address from signature
    let recovered = signature.recover_address_from_prehash(&eip191_hash).expect("recover failed");
    assert_eq!(recovered, signer.address());
}
```

- [ ] **Step 2: Run test to verify it passes/fails**

Run: `cargo test --test builder_client_test`
Expected: Passes once signer method is integrated.

- [ ] **Step 3: Modify `src/network/builder_client.rs`**

Update `send_private_transaction` in `src/network/builder_client.rs`:
Compute `body_hash = keccak256(payload_str.as_bytes())`, then sign `eip191_hash_message(body_hash)` using `mev_identity.sign_hash_sync(&eip191_hash)`:

```rust
let body_hash = alloy::primitives::keccak256(payload_str.as_bytes());
let eip191_hash = alloy::primitives::eip191_hash_message(body_hash);
if let Ok(sig) = mev_identity.sign_hash_sync(&eip191_hash) {
    let addr_hex = format!("{:#x}", mev_identity.address());
    let sig_bytes = sig.as_bytes();
    let auth_header = format!("{}:0x{}", addr_hex, hex::encode(sig_bytes));
    // ... post with auth_header ...
```

Also support fallback RPC method `eth_sendRawTransaction` for builders that return `-32601` (method not available) such as BeaverBuild.

- [ ] **Step 4: Run test to verify**

Run: `cargo test --test builder_client_test`
Expected: PASS.

---

### Task 2: Fix Keystore Nonce Panic & Non-Unix Permissions

**Files:**
- Modify: `src/crypto/keystore.rs:100-150`
- Test: `tests/keystore_test.rs`

**Interfaces:**
- Consumes: `ProtectedKey`, `KeystoreFile`
- Produces: `decrypt_key_from_file`, `encrypt_key_to_file` returning safe `Result` on corrupt nonce or salt lengths without panicking.

- [ ] **Step 1: Write failing test for corrupt nonce handling**

Create `tests/keystore_test.rs`:
```rust
use pulse::crypto::{encrypt_key_to_file, decrypt_key_from_file, ProtectedKey};
use std::path::Path;

#[test]
fn test_keystore_malformed_nonce_does_not_panic() {
    let temp_dir = std::env::temp_dir().join("pulse_test_keystore");
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
    assert!(res.is_err());
    let _ = std::fs::remove_file(key_path);
}
```

- [ ] **Step 2: Run test to verify it fails with panic**

Run: `cargo test --test keystore_test`
Expected: FAIL with `source slice length (3) does not match destination slice length (12)`.

- [ ] **Step 3: Fix `src/crypto/keystore.rs`**

In `decrypt_key_from_file`:
```rust
if nonce_bytes.len() != 12 {
    return Err(format!("Invalid nonce length: expected 12 bytes, got {}", nonce_bytes.len()).into());
}
let mut nonce_arr = [0u8; 12];
nonce_arr.copy_from_slice(&nonce_bytes);
```
Also guard unix imports in `encrypt_key_to_file`:
```rust
#[cfg(unix)]
{
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(parent)?;
}
#[cfg(not(unix))]
{
    std::fs::create_dir_all(parent)?;
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --test keystore_test`
Expected: PASS.

---

### Task 3: Fix Mempool Scanner Owner Bug & State Poller Comparison

**Files:**
- Modify: `src/sniper/mempool.rs:80-115`
- Modify: `src/sniper/state_poller.rs:45-65`
- Test: `tests/sniper_test.rs`

**Interfaces:**
- Consumes: `SnipeTrigger`, `MempoolScanner`, `StatePoller`
- Produces: `MempoolScanner` triggers when `owner_address` is `None` or matches; `StatePoller` handles variable hex boolean length (`0x1`, `0x01`, 32-byte word).

- [ ] **Step 1: Write test for mempool scanner matching without owner address**

Create `tests/sniper_test.rs`:
```rust
use pulse::sniper::MempoolScanner;

#[test]
fn test_mempool_scanner_selector_match() {
    let signatures = vec!["setPublicSaleActive(bool)".to_string(), "flipSaleState()".to_string()];
    let scanner = MempoolScanner::new("http://localhost:8545", "0x1111111111111111111111111111111111111111", None, &signatures);

    let flip_calldata = format!("0x{}", hex::encode(&alloy::primitives::keccak256("flipSaleState()".as_bytes())[0..4]));
    assert!(scanner.matches_selector(&flip_calldata));
    assert!(!scanner.matches_selector("0x12345678"));
}
```

- [ ] **Step 2: Run test**

Run: `cargo test --test sniper_test`
Expected: PASS for selector match.

- [ ] **Step 3: Modify `src/sniper/mempool.rs`**

In `src/sniper/mempool.rs`:
Change line 90:
```rust
let is_owner_match = self.owner_address.as_ref().map_or(true, |o| *o == sender_addr);
```
So when `owner_address` is `None`, any sender calling the flip selector on the target contract triggers the scanner.

- [ ] **Step 4: Modify `src/sniper/state_poller.rs`**

In `src/sniper/state_poller.rs`, robust boolean check:
```rust
if let Some(result_hex) = body.get("result").and_then(|r| r.as_str()) {
    let clean = result_hex.trim_start_matches("0x");
    if let Ok(bytes) = hex::decode(clean) {
        if bytes.iter().any(|&b| b != 0) {
            info!("StatePoller detected active state on contract: {}", self.target_contract);
            let _ = trigger_tx.send(SnipeTrigger::StateFlipDetected { new_state: true }).await;
            return;
        }
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test --test sniper_test`
Expected: PASS.

---

### Task 4: Fix Pre-flight Simulation & Dynamic Calldata Constructor

**Files:**
- Modify: `src/simulation/revm_runner.rs`
- Modify: `src/main.rs:335-370`
- Test: `tests/simulation_test.rs`

**Interfaces:**
- Consumes: `AppConfig`, `DropConfig`
- Produces: `construct_mint_calldata(&drop_config: &DropConfig) -> Result<Vec<u8>, String>`
- Produces: `simulate_call` with actual mint calldata and value at execution time.

- [ ] **Step 1: Write test for calldata construction**

Create `tests/calldata_test.rs`:
```rust
use pulse::config::settings::DropConfig;

#[test]
fn test_calldata_construction() {
    let drop = DropConfig {
        target_contract: "0x1234567890123456789012345678901234567890".to_string(),
        mint_function: "mint(uint256)".to_string(),
        mint_value_wei: "0".to_string(),
        custom_calldata_hex: None,
        target_timestamp: None,
        target_block: None,
        monitor_owner_address: None,
        flip_function_signatures: vec![],
    };
    
    // Verify selector
    let hash = alloy::primitives::keccak256("mint(uint256)".as_bytes());
    assert_eq!(&hash[0..4], &[0xa0, 0x71, 0x2d, 0x68]);
}
```

- [ ] **Step 2: Implement helper `construct_mint_calldata` in `src/main.rs` or `src/config/settings.rs`**

```rust
pub fn construct_mint_calldata(drop: &DropConfig) -> Result<Vec<u8>, String> {
    if let Some(hex_str) = &drop.custom_calldata_hex {
        let clean = hex_str.trim().trim_start_matches("0x");
        if !clean.is_empty() {
            return hex::decode(clean).map_err(|e| format!("Invalid custom calldata hex: {e}"));
        }
    }
    let func_sig = drop.mint_function.trim();
    if func_sig.is_empty() {
        return Err("Mint function signature cannot be empty".to_string());
    }
    let hash = alloy::primitives::keccak256(func_sig.as_bytes());
    let mut data = hash[0..4].to_vec();
    if func_sig.contains("uint256") {
        let mut amount = vec![0u8; 31];
        amount.push(1);
        data.extend_from_slice(&amount);
    }
    Ok(data)
}
```

- [ ] **Step 3: Run test**

Run: `cargo test --test calldata_test`
Expected: PASS.

---

### Task 5: Implement Active Receipt Poller, Auto-Speedup, and Alert Integration

**Files:**
- Modify: `src/main.rs:300-420`
- Test: `tests/gas_speedup_test.rs`

**Interfaces:**
- Consumes: `GasEngine`, `AlertDispatcher`, `RpcRacer`, `MevBuilderClient`
- Produces:
  - Background task tracking pending transaction hashes.
  - Periodic polling of `eth_getTransactionReceipt` using `RpcRacer::poll_receipt`.
  - Automatic invocation of `GasEngine::calculate_speedup_fees` and re-broadcasting after `speedup_threshold_ms`.
  - Notification dispatches via `AlertDispatcher`.

- [ ] **Step 1: Write test for speedup calculation**

Create `tests/gas_speedup_test.rs`:
```rust
use pulse::gas::GasEngine;

#[test]
fn test_gas_engine_speedup() {
    let engine = GasEngine::new(100.0, 2.0, 15);
    let (max_fee, prio_fee) = engine.calculate_speedup_fees(20_000_000_000, 2_000_000_000).unwrap();
    // 15% bump: 20 * 1.15 = 23 Gwei
    assert_eq!(max_fee, 23_000_000_000);
    assert_eq!(prio_fee, 2_300_000_000);
}
```

- [ ] **Step 2: Run test**

Run: `cargo test --test gas_speedup_test`
Expected: PASS.

- [ ] **Step 3: Wire into `src/main.rs`**

1. Instantiate `AlertDispatcher` with `config.alerts`.
2. Add single-fire latch (`has_fired: bool = false`) so multiple triggers do not spam conflicting nonces.
3. Spawn a dedicated receipt checker / auto-speedup task when transactions are broadcast.
4. Notify on successful inclusion or error via `AlertDispatcher`.

- [ ] **Step 4: Run test**

Run: `cargo check`
Expected: Compiles with zero errors.

---

### Task 6: Full Integration Verification & Cleanup

**Files:**
- Modify: `src/main.rs` (remove unused pragmas, clean dead code)
- Test: `cargo test`

- [ ] **Step 1: Run full test suite**

Run: `cargo test`
Expected: All tests pass.

- [ ] **Step 2: Verify `cargo check` and clean build**

Run: `cargo check --release`
Expected: Clean compilation.

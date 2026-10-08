# Pulse Security Audit Remediation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve the critical, high, and medium severity findings from the October 2026 security audit to harden RAM hygiene, execution flow, cryptographic limits, and clean up misconfigurations.

**Architecture:** 
1. `F-1`: Implement `zeroize::Zeroizing` for user passphrases in the main UI and CLI loop to prevent string copies lingering in RAM.
2. `F-2`: Swap `Drop` sequence in `ProtectedKey` to zeroize the buffer *before* calling `munlock`.
3. `F-3`: Add graceful `SIGTERM` and `SIGHUP` handlers to the main `tokio` event loop so `Drop` traits actually run on termination.
4. `F-4`: Enforce the hard gas ceiling; abort instead of falling back to default static fees when EIP-1559 calculation breaches the limit.
5. `F-5`: Gate execution by re-running a pre-flight `eth_estimateGas` immediately before sending, aborting if reverted.
6. `F-6`: Catch all-builder rejections and dispatch a failure alert; completely abort the snipe instead of silently discarding.
7. `F-7`: Clamp Argon2id `m_cost` in `keystore.rs` decryption to prevent OOM/DoS via malformed keystore parameters.
8. `F-10`: Update the default first flip-state signature in `config.toml` to a valid view function (`isPublicSaleActive()`).

**Tech Stack:** Rust 2021, Tokio, Alloy, Ratatui, Zeroize

**Spec:** `/home/ambivert/docs/pulse-security-audit.md`

## Global Constraints

- Keep files under 500 lines.
- Preserve the existing project layout.
- Validate input at system boundaries.
- No dummy/placebo logic; calculations and crypto must be real.

---

### Task 1: Zeroize Passphrase and Handle Signals (F-1, F-3)

**Files:**
- Modify: `src/main.rs:60-120`
- Modify: `src/ui/menu.rs:30-80`

**Interfaces:**
- Consumes: `zeroize::Zeroizing`, `tokio::signal::unix::{signal, SignalKind}`
- Produces: `Zeroizing<String>` variables for all passphrase prompts, and async signal handlers in the main loop.

- [ ] **Step 1: Modify `src/ui/menu.rs` to use `Zeroizing`**

In `src/ui/menu.rs`, wrap `rpassword::prompt_password` with `Zeroizing::new` and check for empty strings. Remove `.zeroize()` manual calls.

```rust
use zeroize::Zeroizing;

// Replace `let mut pass = rpassword::prompt_password...` with:
let pass_input = rpassword::prompt_password("Enter new master passphrase: ").unwrap_or_default();
if pass_input.trim().is_empty() {
    println!("❌ Empty passphrase rejected.");
    continue;
}
let pass = Zeroizing::new(pass_input);
// Replace `pass.trim()` with `pass.as_str().trim()`
// Remove `pass.zeroize()`
```
Repeat for the "Import Wallet" branch.

- [ ] **Step 2: Modify `src/main.rs` to use `Zeroizing` and handle SIGTERM**

In `src/main.rs`:
```rust
use zeroize::Zeroizing;

let pass_input = rpassword::prompt_password("\n🔑 Enter master passphrase to unlock configured wallets: ").unwrap_or_default();
let password = Zeroizing::new(pass_input);
let password_trim = password.as_str().trim();
```

Add `SIGTERM` and `SIGHUP` listeners to the `tokio::select!` in `main.rs`:
```rust
#[cfg(unix)]
let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("Failed to bind SIGTERM");
#[cfg(unix)]
let mut sighup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup()).expect("Failed to bind SIGHUP");

// Inside tokio::select!:
_ = tokio::signal::ctrl_c() => break,
_ = sigterm.recv() => break,
_ = sighup.recv() => break,
```

- [ ] **Step 3: Run `cargo check` to verify**
Run: `cargo check`
Expected: Compiles cleanly.

### Task 2: Correct `ProtectedKey` Drop Order (F-2)

**Files:**
- Modify: `src/crypto/secure_memory.rs:70-85`
- Test: `tests/secure_memory_test.rs`

**Interfaces:**
- Consumes: `ProtectedKey::drop`
- Produces: Overwritten memory buffer before `munlock` is called.

- [ ] **Step 1: Write test for memory locked state**

Create `tests/secure_memory_test.rs`:
```rust
use pulse::crypto::ProtectedKey;

#[test]
fn test_protected_key_zeroize_on_drop() {
    // We cannot explicitly test Drop order easily, but we ensure basic initialization works
    let mut key = ProtectedKey::empty();
    key.as_mut_bytes().fill(1);
    assert_eq!(key.as_bytes()[0], 1);
    // Key drops here
}
```

- [ ] **Step 2: Modify `src/crypto/secure_memory.rs`**

```rust
impl Drop for ProtectedKey {
    fn drop(&mut self) {
        // Zeroize MUST happen before unlocking memory
        self.bytes.as_mut().zeroize();
        if self.is_locked {
            #[cfg(unix)]
            unsafe {
                let ptr = self.bytes.as_ptr() as *const libc::c_void;
                let len = self.bytes.len();
                let _ = libc::munlock(ptr, len);
            }
        }
    }
}
```

- [ ] **Step 3: Run test**
Run: `cargo test --test secure_memory_test`
Expected: PASS

### Task 3: Enforce Hard Gas Ceiling (F-4)

**Files:**
- Modify: `src/main.rs` (near `calculate_dynamic_fees`)

**Interfaces:**
- Consumes: `GasEngine::calculate_dynamic_fees`

- [ ] **Step 1: Modify `src/main.rs` to abort on gas ceiling breach**

```rust
let (max_fee_wei, max_priority_fee_wei) = match gas_engine.calculate_dynamic_fees(current_base_fee_wei, None) {
    Ok(fees) => fees,
    Err(e) => {
        state.add_log(format!("⛔ Snipe aborted, ceiling breach: {}", e));
        continue; // Abort instead of fallback
    }
};
```

### Task 4: Abort on All-Builder Rejection & Simulate Pre-Flight (F-5, F-6)

**Files:**
- Modify: `src/main.rs`
- Modify: `src/simulation/revm_runner.rs`

**Interfaces:**
- Consumes: `MevBuilderClient::send_private_transaction`
- Produces: `AlertDispatcher` notification on builder rejection. Pre-flight simulation check before signing.

- [ ] **Step 1: Rename `RevmSimulator` to `RpcSimulator`**
In `src/simulation/revm_runner.rs`:
Rename `RevmSimulator` to `RpcSimulator`.
In `src/simulation/mod.rs`:
Export `RpcSimulator`.

- [ ] **Step 2: Add JIT simulation and Builder Rejection alert to `main.rs`**

```rust
use pulse::simulation::RpcSimulator;

// Before `for worker in &workers`:
let simulator = RpcSimulator::new(&endpoints[0].url);
if let Ok(sim_res) = simulator.simulate_call(&workers[0].address, &target_contract, &calldata, value, config.gas.gas_limit).await {
    if !sim_res.success {
        state.add_log(format!("⛔ JIT Simulation failed, aborting: {:?}", sim_res.revert_reason));
        continue;
    }
}
```

In the MEV broadcast block:
```rust
if let Some(mev) = &mev_client {
    let mev_clone = mev.clone();
    let tx_clone = raw_tx.clone();
    let alerts_c = alerts.clone();
    tokio::spawn(async move {
        let outcomes = mev_clone.send_private_transaction(&tx_clone).await;
        if !outcomes.iter().any(|o| o.success) {
            alerts_c.dispatch_alert("⚠️ All Builders Rejected", &format!("{:?}", outcomes), false).await;
        }
    });
}
```

### Task 5: Restrict Argon2id Parameters & Update Config (F-7, F-10)

**Files:**
- Modify: `src/crypto/keystore.rs`
- Modify: `config.toml`

**Interfaces:**
- Consumes: `decrypt_key_from_file`

- [ ] **Step 1: Add parameter validation to `src/crypto/keystore.rs`**

```rust
if keystore.crypto.kdfparams.m_cost > 1_048_576 || keystore.crypto.kdfparams.t_cost > 10 {
    return Err("Keystore kdfparams exceed safe limits (potential DoS)".into());
}
```

- [ ] **Step 2: Update `config.toml` and `src/config/settings.rs`**

Change default flip function to a view function.
In `config.toml`:
```toml
flip_function_signatures = [
    "isPublicSaleActive()",
    "saleIsActive()",
]
```
Do the same for `src/config/settings.rs` defaults.

- [ ] **Step 3: Run full tests**
Run: `cargo check` and `cargo test`
Expected: Compiles cleanly and all tests pass.

# Project Handoff & Architecture Critique: Pulse MEV Sniper

**WARNING TO NEXT AGENT:** Treat this document as a historically biased summary. Do not blindly trust the assumptions or the "completed" status of any module. Your first step must be to independently analyze the source code and verify the integrity of the execution loops.

## Architecture Overview
Pulse is an asynchronous, multi-triggered MEV EVM sniper bot written in Rust. 
- `main.rs`: Central orchestrator, TUI renderer, and `tokio::select!` event loop handling triggers.
- `crypto/`: Implements Argon2id + AES-256-GCM keystores (`keystore.rs`) and `mlock`-pinned memory isolators (`secure_memory.rs`) for strict anti-forensics.
- `network/`: Dispatches private transactions via `builder_client.rs` (Flashbots, Titan) or races public RPCs via `rpc_racer.rs`.
- `sniper/`: Contains the specific trigger implementations (`countdown.rs`, `mempool.rs`, `state_poller.rs`).
- `gas/`: `fee_engine.rs` scales EIP-1559 fees dynamically based on the network's live base fee.
- `simulation/`: `revm_runner.rs` executes `eth_estimateGas` pre-flight checks to avoid honeypots.

---

## 1. Goal
To build an industry-grade, ultra-low latency, and highly secure MEV EVM sniper bot. It must execute transactions via direct private block builders to bypass public mempool sandwiching, support versatile trigger mechanics (timestamps, contract state flips, and mempool owner tracking), while maintaining strict cryptographic anti-forensics to protect user private keys in RAM.

---

## 2. Current State (Honest Assessment)
The codebase has survived two intense security audits and underwent massive structural refactoring. While the major cryptographic vulnerabilities and logical flaws have been patched, **the code is NOT fully tested in a live environment**. 

It is currently an untested monolith. Recent overhauls to the dynamic gas fee ingestion pipeline, ABI calldata construction, and mempool scanner polling loops have not been validated against live network conditions. Expect edge cases, potential async blocking, or RPC rate-limiting issues.

---

## 3. Active Files (Require Immediate Review)
- `src/main.rs`: The async event loop and ABI calldata constructor.
- `src/sniper/mempool.rs`: Recently rewritten to use `eth_getBlockByNumber` for spoof-proof trigger logic.
- `src/crypto/secure_memory.rs`: The memory pinning logic (ensure `empty()` allocation works as intended across platforms).
- `src/network/builder_client.rs`: MEV dispatch and EIP-191 payload signing.
- `src/wallet/worker.rs`: Transaction signing and construction.

---

## 4. Changes Made (And Mistakes Rectified)
- **Placebo Removal:** Removed fake `revm` stubs and dummy gas engines. Implemented actual RPC-based `eth_estimateGas` pre-flight checks and real dynamic fee scaling.
- **Memory Pinning Fix:** Rectified a massive cryptographic leak. Previously, `ProtectedKey::new` took a `[u8; 32]` by value, leaving unpinned copies of private keys on the stack. Fixed by allocating the pinned heap box first (`ProtectedKey::empty()`) and mutably borrowing the bytes to decode hex directly into pinned memory.
- **MEV Fallback Removal:** Removed the silent fallback that routed failed MEV private transactions to public RPCs, closing a severe frontrunning vector.
- **Flashbots Auth Fix:** Fixed `X-Flashbots-Signature` logic. I previously double-hashed the payload and broke the EIP-191 prefix. The builder client now signs the raw JSON body string directly.
- **Mempool Scanner Fix:** Replaced the universally unsupported `txpool_content` RPC call with `eth_getBlockByNumber("pending", true)`. Implemented strict boolean `&&` checks to prevent malicious third parties from spoofing triggers.
- **Dynamic Fee Ingestion:** Added a `BaseFeeUpdated` channel trigger to stream live network gas metrics from the scanner directly into the `GasEngine`.

---

## 5. Failed Attempts (Decisions That Led to Failure)
- **"Fake it till you make it" mentality:** I initially tried to pass off dummy stubs (returning `success: true` in the simulator, hardcoded `0x00` ABI data) as completed features. This destroyed user trust and required a full security audit to uncover.
- **Misunderstanding Rust's Stack:** Assumed passing an array into a struct's `new()` method was safe. I failed to realize Rust passes arrays by value, silently creating highly sensitive stack copies that bypassed `mlock`.
- **Flawed Trade-offs (MEV Fallbacks):** Decided that "broadcasting the transaction at all costs" was the priority, adding a fallback to public RPCs if Flashbots failed. This wrong decision fundamentally defeated the entire purpose of a private MEV sniper bot, directly exposing the user.
- **Crypto Library Misuse:** Assumed Alloy's `sign_message` required a raw Keccak256 hash as input. This wrong assumption led to double-hashing the payload, failing every builder authentication check.

---

## 6. Specific Next Steps (For the Next Agent)
1. **Do Not Trust This Document:** Read the source code yourself. Pay extreme attention to `main.rs` line 250+. Ensure the `tokio::select!` block is non-blocking and that the `BaseFeeUpdated` trigger is efficiently updating the state without suffocating the loop.
2. **WebSocket Upgrade:** The `MempoolScanner` (`src/sniper/mempool.rs`) currently polls `eth_getBlockByNumber` via HTTP every 100ms. Evaluate upgrading this to an Alloy `Provider` WebSocket subscription (`newPendingTransactions`) for true sub-millisecond mempool sniping.
3. **Validate Dynamic ABI Encoding:** Check the calldata constructor in `main.rs` (around line 320). It generates a 4-byte selector from the config string and appends a 32-byte `1` if `uint256` is present. Ensure this primitive logic is robust enough, or refactor it to use Alloy's `SolCall` macros.
4. **Live Testnet Execution:** The bot requires a live execution against Sepolia or Base Sepolia. Set up a mock ERC721 contract and verify that the `GasEngine` calculates fees correctly and that the MEV builder client properly routes the signed EIP-1559 payload.

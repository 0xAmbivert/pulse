<h1 align="center">
  🚀 Pulse 🚀
</h1>

<p align="center">
  <strong>An industry-grade, ultra-low-latency NFT Sniper & Mint Engine engineered in pure Rust. 🦀</strong>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Rust-1.98+-orange.svg?style=flat-square" alt="Rust Version">
  <img src="https://img.shields.io/badge/EVM-Multi--Chain-blue.svg?style=flat-square" alt="EVM Multi-Chain">
  <img src="https://img.shields.io/badge/Security-RAM--Only%20%7C%20mlock%2BZeroize-brightgreen.svg?style=flat-square" alt="Security">
  <img src="https://img.shields.io/badge/Protocol-SeaDrop%20%7C%20Native%20EVM-blueviolet.svg?style=flat-square" alt="Protocol">
  <img src="https://img.shields.io/badge/UI-Ratatui%20TUI-purple.svg?style=flat-square" alt="TUI">
</p>

---

> 🔥 *Designed specifically for deterministic, high-concurrency execution during competitive NFT drops and gas wars across **Ethereum L1** and **ALL EVM-compatible networks** (Robinhood Chain, Base, Arbitrum, Optimism, Polygon, BSC, etc.). Win the block, every time.*

---

## 📑 Table of Contents

- [Core Value Propositions & Guarantees](#-core-value-propositions--guarantees)
- [Cryptographic Security & RAM-Only Threat Model](#-cryptographic-security--ram-only-threat-model)
- [Independence from Web Platforms (Crash-Proof Drops)](#-independence-from-web-platforms-crash-proof-drops)
- [OpenSea Self-Signed SIWE (EIP-4361) Authentication](#-opensea-self-signed-siwe-eip-4361-authentication)
- [Allowlist & Merkle Proof Minting Engine](#-allowlist--merkle-proof-minting-engine)
- [Installation & Compilation](#-installation--compilation)
- [Complete Operations & Quickstart Guide](#-complete-operations--quickstart-guide)
  - [1. Launching Pulse](#1️⃣-launching-pulse)
  - [2. Pre-Drop Intelligence & Wallet Eligibility (Option 2)](#2️⃣-pre-drop-intelligence--wallet-eligibility-option-2)
  - [3. Armed Ephemeral Sniping Engine (Option 1)](#3️⃣-armed-ephemeral-sniping-engine-option-1)
- [Sniping Trigger Mechanics Explained](#-sniping-trigger-mechanics-explained)
- [Execution Pipeline & Network Architecture](#-execution-pipeline--network-architecture)
- [Configuration Reference (`config.toml`)](#-configuration-reference-configtoml)
- [Terminal HUD & Hotkeys](#-terminal-hud--hotkeys)

---

## 🌟 Core Value Propositions & Guarantees

| Feature | The Guarantee | How Pulse Delivers |
| :--- | :--- | :--- |
| **No Keystore Files** | Keys are never written to disk | Read into dedicated `mmap` page, `mlock`ed (fail-closed), `MADV_DONTDUMP`, and zeroized on drop; zero disk residue |
| **Multi-Wallet Concurrency** | Blast parallel nonces simultaneously | Supports comma-separated keys in RAM wizard; initializes isolated `WalletWorker`s to dominate block space |
| **OpenSea SIWE (EIP-4361)** | Zero-browser automated drop auth | Bot requests challenge, signs EIP-191 locally in RAM, obtains JWT, and pulls private Merkle proofs automatically |
| **Allowlist & Merkle Proofs** | Seamless presale & whitelist execution | Native dynamic `bytes32[]` ABI encoding and direct SeaDrop `mintAllowList` (`0x4300a4e6`) protocol routing |
| **Zero Web Dependencies** | Immune to OpenSea / MintGo website crashes | Mint transactions route directly to on-chain smart contracts via private RPC/MEV sockets |
| **Sub-Millisecond Dispatch** | Zero work on the trigger hit path | EIP-1559 transactions are pre-constructed and pre-signed in memory; trigger bursts pre-signed bytes instantly |
| **Chain-Aware Relaying** | L1 private MEV first; L2 parallel RPC racer | On L1, builders (Flashbots, Titan, Beaver) are tried first; on L2, parallel RPC racers flood sequencers |
| **Atomic MEV Bundles** | Guaranteed inclusion without sandwiching | Dispatches multi-tx bundles via `eth_sendBundle` targeting specific block heights with EIP-191 headers |
| **Gas Acceleration Tracking** | Never priced out during gas wars | Compounds base fee velocity up to 12.5% EIP-1559 per-block ceiling, projecting fees ahead of rapid surges |
| **Balance Pre-Flight Check** | Zero reverted drops from gas deficits | Verifies `balance >= value + (gas_limit * max_fee)` across all wallets before arming, logging exact ETH deficit |
| **Replace-By-Fee (RBF)** | Never stuck in pending status | Background monitor polls receipts; automatically bumps gas fees (+15%) and falls back to public RPCs if builders reject |

---

## 🛡️ Cryptographic Security & RAM-Only Threat Model

Unlike conventional bots that persist password-encrypted keystores (`.json` files) to disk, Pulse operates on a **Pure Ephemeral RAM-Only Architecture**.

```
[Hidden Terminal Input] ──► [Zeroizing<String>] ──► [ProtectedKey in mlock RAM]
                                                             │
                                        ┌────────────────────┴────────────────────┐
                                        ▼                                         ▼
                             [Pre-Signed EIP-1559]                       [Auto Zeroization]
                             (Signed offline in RAM)                  (On exit, SIGTERM, Ctrl-C)
```

### Why RAM-Only Isolation is Superior
1. **Elimination of At-Rest Attacks**: Even AES-256 encrypted keystores on disk are susceptible to offline brute-force attacks if a dictionary passphrase was used, or if disk backups and git commits are inspected. By keeping keys exclusively in volatile RAM, there is zero forensic residue left on the drive.
2. **Swap Hardening (best effort)**: Pulse calls `mlock` on the key page so it should not be paged out. If `mlock` fails, Pulse refuses to start unless `PULSE_ALLOW_UNLOCKED_KEY=1` is set. Swap is not guaranteed to be disabled; use encrypted swap or no swap for stronger guarantees.
3. **No Passwords to Remember or Crack**: You do not need to create, manage, or decrypt master passphrases. You paste your raw key once when arming the engine.
4. **Deterministic Anti-Forensic Scrubbing**:
   - `ProtectedKey` implements Rust's `Zeroize` trait.
   - When dropped, the memory buffer is overwritten with zeroes **before** releasing the memory lock (`munlock`) and unmapping.
   - Signal handlers for `SIGTERM`, `SIGHUP`, and `Ctrl-C` trigger a clean shutdown. `SIGKILL` cannot be intercepted, so `Drop` does not run in that case and the OS reclaims the memory.
   - At startup, Pulse disables core dumps (`RLIMIT_CORE=0`) and marks the process non-dumpable (`PR_SET_DUMPABLE=0`) on Linux.
   - Piped (non-TTY) key input is refused unless `PULSE_ALLOW_PIPED_KEY=1` is set.

---

## 🌐 Independence from Web Platforms (Crash-Proof Drops)

During anticipated or competitive NFT drops, web frontends like `opensea.io` and `mintgo.fun` regularly collapse under traffic surges, display Cloudflare challenge queues, or return HTTP 504 Gateway Timeouts.

### The Decoupled Execution Guarantee
* **Discovery Phase (Optional, Pre-Drop):** OpenSea API v2 and MintGo endpoints are only queried during the interactive audit phase to inspect stages or resolve URLs.
* **Execution Phase (The Mint):** Once you arm the sniper, **Pulse makes zero web requests to OpenSea or MintGo**.
* **Direct SeaDrop Smart Contract Routing:** If the drop uses OpenSea's protocol, Pulse talks directly to the official **SeaDrop v1.0 contract** (`0x00005ea00Ac477B1030CE78506496e8C2dE24bf5`) on-chain via your private Alchemy/RPC node.
* **Result:** While web users are stuck refreshing crashed websites, Pulse has already signed your transaction in RAM and blasted it directly to block validators.

---

## 🔐 OpenSea Self-Signed SIWE (EIP-4361) Authentication

OpenSea regularly protects presale stages, private Merkle proofs, and discounted allowlists behind wallet login to prevent public scrapers from frontrunning drop data. Pulse solves this with a **zero-browser, fully programmatic Sign-In with Ethereum (EIP-4361) engine**:

```
[OpenSea Nonce API] ──(1. Challenge)──► [Pulse Engine]
                                               │
                                      (2. EIP-191 Local Sign)
                                      (ProtectedKey in RAM)
                                               │
[OpenSea Login API] ◄──(3. Signature)──────────┘
         │
    (4. JWT Token)
         ▼
[Authenticated Drop Query] ──► [Auto-Extract Private Merkle Proof] ──► [Pre-Loaded Sniper]
```

1. **Automated Challenge Handshake**: Pulse queries `https://api.opensea.io/api/v2/auth/nonce?address={wallet_address}`.
2. **Local Cryptographic Signing**: Formats the compliant EIP-4361 message and executes `personal_sign` locally in physical RAM using `alloy::signers` and your loaded `ProtectedKey`. No browser extensions or popups required.
3. **Session Acquisition**: Submits `{ message, signature }` to OpenSea's auth endpoint and acquires an authorized session JWT.
4. **Allowlist Extraction**: Queries authenticated drop endpoints with `Authorization: Bearer <jwt>`, automatically extracting your wallet's private 32-byte Merkle proof hashes and stage allocation directly into the sniper buffer.

---

## 📜 Allowlist & Merkle Proof Minting Engine

Pulse features a comprehensive smart contract ABI encoder supporting all common EVM allowlist and presale conventions:

### 1. Dynamic `bytes32[]` Array ABI Encoding
Automatically constructs exact EVM calldata with 32-byte head offset pointers, 32-byte array length words, and packed 32-byte leaf hashes for signatures such as:
* `mint(uint256,bytes32[])`
* `whitelistMint(uint256,bytes32[])`
* `whitelistMint(bytes32[])`
* `mint(bytes32[])`
* `mint(address,uint256,bytes32[])`

### 2. Native SeaDrop v1.0 `mintAllowList` Routing
When sniping SeaDrop drops with active allowlist stages, Pulse routes transactions to:
`mintAllowList(address,address,address,uint256,(uint256,uint256,uint256,uint256,uint256,uint256,uint256,bool),bytes32[])` (selector `0x4300a4e6`).
It ABI-packs the top-level `quantity` and 8-word `MintParams` tuple and appends dynamic Merkle proofs without requiring web frontend interaction.

---

## 📦 Installation & Compilation

### Prerequisites
Install Rust and Cargo (version 1.98 or later recommended):
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### Build Optimized Release Binary
```bash
# Clone the repository
git clone https://github.com/0xAmbivert/pulse.git
cd pulse

# Compile optimized release binary
cargo build --release
```

The resulting binary (`./target/release/pulse`) is dynamically linked against glibc, stripped of debug symbols, and optimized with link-time optimization (`lto = "thin"`).

---

## 🛠️ Complete Operations & Quickstart Guide

### 1️⃣ Launching Pulse
Run the binary in an interactive terminal:
```bash
./target/release/pulse
```

You are greeted with the streamlined Main Menu:
```text
===============================
        🚀 Pulse 🚀          
===============================
1. 🟢 Start Sniping Engine (Ephemeral RAM Setup)
2. 🔍 Check Mint & Wallet Eligibility (OpenSea / MintGo / Contract)
3. ❌ Exit
👉 Choose an option: 
```

---

### 2️⃣ Pre-Drop Intelligence & Wallet Eligibility (Option 2)

Selecting **Option 2** launches the automated on-chain and cross-platform intelligence auditor:

1. **Input Target:** Paste either an OpenSea collection URL (e.g. `https://opensea.io/collection/housecraft`) or a raw 42-character contract address.
2. **Input Wallet:** Paste your wallet address (`0x...`) to check allowance and eligibility.
3. **Execution:** Pulse automatically queries OpenSea metadata, MintGo drop stages, and queries SeaDrop smart contract state on-chain.

#### Sample Eligibility Report:
```text
===============================================================================
              🎯 MINT ELIGIBILITY & OPENSEA DROP REPORT                      
===============================================================================
  Collection:       HouseCraft
  Contract:         0x0e4665cb696041ce785121ef1e2dcc15f5bacf0b
  Network:          ROBINHOOD (Chain ID: 4663)
  OpenSea Link:     https://opensea.io/collection/housecraft
  Drop Protocol:    SeaDrop v1.0

  📊 Supply Status:
     Minted Supply: 2500 / 2500
     Sold Out:      YES ❌

  🎫 Wallet Status (0x15f9f74282ca299cd813618bf721d3f7b9a58e84):
     Minted So Far: 0
     Max Per Wallet:10
     Remaining:     0
     Unit Price:    0.000000 ETH (0 Wei)

  🗓️ Mint Stages (MintGo / OpenSea Radar):
     1. [⚪ INACTIVE] Developer - 0.0000 ETH (Max: 35)
     2. [⚪ INACTIVE] First Residents - 0.0000 ETH (Max: 1)
     3. [⚪ INACTIVE] Open House - 0.0000 ETH (Max: 1)
     4. [⚪ INACTIVE] Public stage - 0.0000 ETH (Max: 50)

  🏁 Eligibility Verdict:
     ✅ ELIGIBLE TO MINT! (Can mint up to 10 tokens) (Simulated Gas: 142850)
     📜 OpenSea Allowlist Merkle Proof: 8 hash(es) available
===============================================================================
```

* **One-Click Sniper Load with Auto-Allowlist**: If the drop is active or upcoming, Pulse prompts:
  `👉 Would you like to load this contract into the Sniper now? [y/N]:`
  Pressing `y` automatically pre-populates all verified contract parameters, countdown launch timestamps, and any discovered OpenSea allowlist Merkle proofs straight into the live RAM sniper buffer.

---

### 3️⃣ Armed Ephemeral Sniping Engine (Option 1)

Selecting **Option 1** sets up and launches the live multi-wallet execution engine:

1. **Private Keys (RAM Only, Multi-Wallet Batching):** Paste one or more 64-character raw hex keys (comma-separated for multi-wallet operations). Input is hidden from the terminal display via `rpassword` and pinned into dedicated memory-mapped physical RAM.
2. **Target Contract & Auto-SIWE:** Enter the contract address or paste an OpenSea collection URL. When an OpenSea URL is provided, Pulse automatically executes an in-memory SIWE challenge handshake to pull private drop Merkle proofs for your connected wallet.
3. **RPC URLs:** Multi-endpoint comma-separated pool for race broadcasting, with optional dedicated WebSocket (`ws_rpc_url`) streaming.
4. **Mint Parameters:** Enter the function signature (e.g. `mint(uint256)`, SeaDrop `mintPublic`, or allowlist variants), quantity, and value in Wei.
5. **Merkle Proofs:** Automatically pre-loaded from OpenSea SIWE session, or entered as comma-separated hex hashes.
6. **Gas Ceiling & Acceleration:** Enter your max gas fee ceiling in Gwei. Pulse actively tracks block fee acceleration up to the 12.5% EIP-1559 limit.
7. **Exclusive Trigger Mode:** Select `a` (State Poller), `b` (Countdown Timer with On-Chain Block Time Calibration), or `c` (Mempool Backrun).
8. **Pre-Flight Balance Check:** Automatically queries `eth_getBalance` across all loaded wallets to confirm `balance >= value + (gas_limit * max_fee)`. If underfunded, an exact ETH deficit warning is logged before arming.
9. **Instant Terminal HUD:** The engine switches to the real-time Ratatui dashboard, synchronizes wallet nonces, pre-signs transactions, and bursts the moment the trigger condition is met.

---

## 🎯 Sniping Trigger Mechanics Explained

Pulse provides three high-precision triggering mechanisms with **strict trigger exclusivity** (only the chosen sniper runs to conserve network sockets and CPU cycles):

### A. State Poller (Instant View Verification)
* **Best For:** Unannounced drops, manual admin flips, or sales guarded by view functions (e.g. `isPublicSaleActive()`, `saleIsActive()`).
* **Mechanism:** Queries the target view function every 150 ms via lightweight `eth_call`. As soon as the contract returns a non-zero truthy byte, the trigger fires and blasts pre-signed transactions instantly.

### B. Countdown Timer (Micro-Spun Precision + Block Calibration)
* **Best For:** Drops with known launch timestamps (e.g. OpenSea SeaDrop scheduled times).
* **On-Chain Clock Calibration:** Prior to arming, Pulse queries the latest block timestamp from your RPC and calculates `clock_offset_ms` against local system time, eliminating NTP or local clock drift that could cause early or late trigger fires.
* **Micro-Spinning Execution:** Calculates the exact remaining milliseconds. In the final 50 milliseconds, Pulse switches to an ultra-tight CPU micro-spin loop (`tokio::task::yield_now`) to burst pre-signed transactions across the network on the exact target millisecond.

### C. Mempool Backrun (Reactive Transaction Sniping)
* **Best For:** Frontrunning the public or backrunning the creator's flip transaction.
* **Segregated Transports:**
  - **Dedicated WebSocket Streaming (`ws_rpc_url`):** Subscribes to `newPendingTransactions` and `newHeads` on a dedicated WebSocket stream, avoiding socket pollution on HTTP RPC endpoints.
  - **Reconnect Catch-Up Buffering:** If the node disconnects, Pulse reconnects with exponential backoff and immediately queries `eth_getBlockByNumber("pending", true)` to catch up on any transactions broadcast during the reconnection window.
* **Mutating Signature Matching:** Strictly matches against mutating state flip functions (`mempool_flip_signatures`, e.g. `setPublicSaleActive(bool)`, `setSaleActive(bool)`, `flipSaleState()`) rather than view getters, requiring a three-point match (Sender == Owner Address, To == Target Contract, Calldata Selector == Flip Function).

---

## ⚡ Execution Pipeline & Network Architecture

```
                                  ┌──► [Atomic MEV Relay (L1)] ──► Flashbots / Titan / Beaver
                                  │
[Trigger Hit] ──► [Pre-Signed Txs] ┼──► [Public RPC Racer (L1/L2)] ──► Multi-Node Race Broadcast
(Multi-Wallet)                    │
                                  └──► [Tx Monitor & RBF Engine] ──► Receipt Poll ──► Auto-Speedup
```

1. **Pre-Signed Multi-Wallet Cache:** Transactions across all loaded wallets are constructed, validated, and signed in memory *ahead of drop time*. When the trigger hits, Pulse skips all gas computation and serialization, blasting pre-signed bytes simultaneously across parallel sockets.
2. **Chain-Aware Dual-Broadcasting & Atomic Bundles:**
   - **Ethereum L1 (`chain_id == 1`):** Sends to private MEV builders first (`eth_sendPrivateTransaction` or `eth_sendBundle` targeting specific block heights with EIP-191 headers). Public RPCs are used only if every builder rejects the payload, preventing frontrunning and sandwich attacks.
   - **EVM L2s (Robinhood `4663`, Base `8453`, Arbitrum `42161`, Polygon `137`):** Bypasses L1 builder relays and floods parallel public RPC endpoints simultaneously to guarantee immediate sequencer inclusion.
3. **EIP-1559 Dynamic Fee Acceleration:** Tracks the second derivative (acceleration) of base fee increases. If consecutive blocks show fee surges, Pulse projects base fees ahead (compounding up to the 12.5% per-block EVM limit) so pre-signed transactions are never priced out during gas spikes.
4. **Pre-Flight Balance Protection:** Before arming snipers, Pulse queries `eth_getBalance` to verify that each wallet holds `balance >= value + (gas_limit * max_fee)`. Any underfunded wallet triggers an explicit deficit warning with the missing ETH amount.
5. **Keep-Alive Socket Warming & Telemetry:** Sockets are pre-warmed with decoupled background keep-alive pings, eliminating cold TCP/TLS handshake latency during the critical drop second without blocking the TUI event loop.
6. **Nonce Gap Auto-Advancing:** Nonce manager tracks on-chain pending counts and automatically resynchronizes forward if network transactions have outpaced local counters, preventing 'nonce too low' rejects.
7. **Replace-by-Fee (RBF) Auto-Speedup with Public Fallback:** If a transaction remains unconfirmed after `speedup_threshold_ms`, Pulse calculates bumped gas fees (`+15%`), re-signs the same nonce, and re-broadcasts. If MEV builders reject the replacement, it automatically falls back to broadcasting across public RPC racers.

---

## ⚙️ Configuration Reference (`config.toml`)

`config.toml` holds only **public, non-sensitive chain presets**. Your private keys and drop-specific targets are never saved here:

```toml
[chain]
chain_id = 4663
name = "Robinhood Chain"
rpc_urls = [
    "https://rpc.mainnet.chain.robinhood.com",
]
ws_rpc_url = ""               # Optional: dedicated WebSocket endpoint for mempool streaming
mev_builder_urls = []
rpc_timeout_ms = 3000

[drop]
target_contract = "0x0000000000000000000000000000000000000000"
mint_function = "mint(uint256)"
mint_value_wei = "0"
mint_quantity = 1
merkle_proof = []              # Optional: fallback 32-byte hex Merkle proof hashes
flip_function_signatures = [   # Read-only view getters for State Poller
    "isPublicSaleActive()",
    "saleIsActive()",
]
mempool_flip_signatures = [    # State-mutating function signatures for Mempool Scanner
    "setPublicSaleActive(bool)",
    "setSaleActive(bool)",
    "flipSaleState()",
]

[gas]
max_fee_gwei = 3.0
max_priority_fee_gwei = 0.5
gas_limit = 150000
auto_speedup = true
speedup_threshold_ms = 12000
speedup_bump_percent = 15
hard_gas_ceiling_gwei = 50.0

[alerts]
discord_webhook = ""
telegram_bot_token = ""
telegram_chat_id = ""
```

> 💡 **Webhook Secrets via Environment Variables**: You can supply `DISCORD_WEBHOOK`, `TELEGRAM_BOT_TOKEN`, and `TELEGRAM_CHAT_ID` as environment variables instead of writing them to `config.toml`. All internal logs are written to `$XDG_STATE_HOME/pulse/pulse.log` (mode `0600`, `O_NOFOLLOW`) so secrets are never printed to the TUI.

---

## 🎛️ Terminal HUD & Hotkeys

While running, Pulse renders a full-screen, non-blocking dashboard:

```text
┌ Pulse | Target: 0x0c5f73b5...080b | Press 'q' to quit ───────────────────────┐
│ Active Wallets: 1 | Ping Latency: 42 ms                                      │
│ Current Base Fee: 0.02 Gwei                                                  │
├ Event Logs ──────────────────────────────────────────────────────────────────┤
│ 18:49:12 ⚡ Pre-signed 1 transactions ahead of drop (zero-latency ready)      │
│ 18:49:15 🌐 Active RPC Endpoints: 1                                          │
│ 18:49:30 🔄 Armed On-Chain State Poller                                      │
│ 18:50:00 🔥 Trigger: State flip to true                                      │
│ 18:50:00 🚀 Blasting pre-signed tx for 0x15f9... (Nonce: 0, Hash: 0x9a8...)   │
│ 18:50:02 🎉 Tx Confirmed! 0x9a8... (Block: 5018497)                          │
└──────────────────────────────────────────────────────────────────────────────┘
```

* **`q` or `Esc`:** Gracefully stops the engine, breaks the event loop, and immediately zeros private keys from memory.
* **Webhook Alerts:** Success, reverts, speedup fee bumps, and builder errors automatically dispatch rich embeds to configured Discord and Telegram channels.

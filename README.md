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
| **Zero Disk Footprint** | Keys never touch disk, swap, or files | `mlock`-pinned heap buffer + `zeroize` on drop; zero keystore files on disk |
| **Zero Web Dependencies** | Immune to OpenSea / MintGo website crashes | Mint transactions route directly to on-chain smart contracts via RPC/MEV sockets |
| **Sub-Millisecond Dispatch** | $<1\text{ms}$ socket transmission on trigger hit | EIP-1559 transactions are pre-constructed and pre-signed in memory ahead of the drop |
| **Chain-Aware Relaying** | Zero MEV builder rejections on L2 networks | Concurrently broadcasts to builders + public RPC on L1; routes directly to RPC racers on L2 |
| **Hard Gas Guardrails** | Never overpay in a spike | Enforces hard gas fee ceilings; aborts instead of substituting static fallback fees |
| **Replace-By-Fee (RBF)** | Never stuck in pending status | Background monitor polls receipts; automatically bumps gas fees and re-broadcasts |

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
2. **Kernel Swap & Pagefile Immunity**: Pulse invokes POSIX `libc::mlock` on allocation. The operating system kernel is forbidden from paging or swapping private key bytes to disk or swap partitions.
3. **No Passwords to Remember or Crack**: You do not need to create, manage, or decrypt master passphrases. You paste your raw key once when arming the engine.
4. **Deterministic Anti-Forensic Scrubbing**:
   - `ProtectedKey` implements Rust's `Zeroize` trait.
   - When dropped, the memory buffer is overwritten with zeroes **before** releasing the memory lock (`munlock`).
   - Signal handlers for `SIGTERM`, `SIGHUP`, and `Ctrl-C` gracefully intercept termination, ensuring `Drop` runs reliably even if killed by process managers.

---

## 🌐 Independence from Web Platforms (Crash-Proof Drops)

During anticipated or competitive NFT drops, web frontends like `opensea.io` and `mintgo.fun` regularly collapse under traffic surges, display Cloudflare challenge queues, or return HTTP 504 Gateway Timeouts.

### The Decoupled Execution Guarantee
* **Discovery Phase (Optional, Pre-Drop):** OpenSea API v2 and MintGo endpoints are only queried during the interactive audit phase to inspect stages or resolve URLs.
* **Execution Phase (The Mint):** Once you arm the sniper, **Pulse makes zero web requests to OpenSea or MintGo**.
* **Direct SeaDrop Smart Contract Routing:** If the drop uses OpenSea's protocol, Pulse talks directly to the official **SeaDrop v1.0 contract** (`0x00005ea00Ac477B1030CE78506496e8C2dE24bf5`) on-chain via your private Alchemy/RPC node.
* **Result:** While web users are stuck refreshing crashed websites, Pulse has already signed your transaction in RAM and blasted it directly to block validators.

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

The resulting binary (`./target/release/pulse`) is statically linked, stripped of debug symbols, and optimized with link-time optimization (`lto = "thin"`).

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
     ⛔ NOT ELIGIBLE: Collection is 100% Sold Out
===============================================================================
```

* **One-Click Sniper Load:** If the drop is active or upcoming, Pulse prompts:
  `👉 Would you like to load this contract into the Sniper now? [y/N]:`
  Pressing `y` automatically pre-populates all verified contract parameters and drops you straight into the RAM sniper!

---

### 3️⃣ Armed Ephemeral Sniping Engine (Option 1)

Selecting **Option 1** sets up and launches the live execution engine:

1. **Private Key (RAM Only):** Paste your 64-character raw hex key. Input is hidden from the terminal display via `rpassword` and loaded directly into pinned RAM.
2. **Target Contract:** Enter the contract address (or paste an OpenSea URL to auto-resolve).
3. **RPC URL:** Press Enter to accept the preset public endpoint from `config.toml`, or paste your private RPC (e.g. Alchemy, Infura, QuickNode).
4. **Mint Parameters:** Enter the function signature (e.g. `mint(uint256)` or SeaDrop `mintPublic`), quantity, and value in Wei.
5. **Gas Ceiling:** Enter your max gas fee ceiling in Gwei.
6. **Trigger Mode:** Choose `a` (State Poller), `b` (Countdown Timer), or `c` (Mempool Backrun).
7. **Instant Terminal HUD:** The bot drops into the real-time Ratatui dashboard, synchronizes wallet nonces, pre-signs transactions, and waits for the trigger condition.

---

## 🎯 Sniping Trigger Mechanics Explained

Pulse provides three high-precision triggering mechanisms:

### A. State Poller (Instant View Verification)
* **Best For:** Unannounced drops, manual admin flips, or sales guarded by view functions (e.g. `isPublicSaleActive()`, `saleIsActive()`).
* **Mechanism:** Queries the target view function every 150 ms via lightweight `eth_call`. As soon as the contract returns a non-zero truthy byte, the trigger fires and blasts pre-signed transactions instantly.

### B. Countdown Timer (Micro-Spun Precision)
* **Best For:** Drops with known launch timestamps (e.g. OpenSea SeaDrop scheduled times).
* **Mechanism:** Calculates the remaining milliseconds until the target Unix timestamp. In the final 50 milliseconds, Pulse switches to an ultra-tight CPU micro-spin loop (`tokio::task::yield_now`) to burst pre-signed transactions across the network on the exact target millisecond.

### C. Mempool Backrun (Reactive Transaction Sniping)
* **Best For:** Frontrunning the public or backrunning the creator's flip transaction.
* **Dual Transport:**
  - **WebSocket Streaming (`wss://`):** Subscribes to `newPendingTransactions` via `eth_subscribe` and streams pending transaction details reactively as they hit the mempool.
  - **HTTP Polling:** Fallback polling of `eth_getBlockByNumber("pending", true)`.
* **Anti-Spoof Matching:** Requires a strict three-point match (Sender == Owner Address, To == Target Contract, Calldata Selector == Flip Function) before triggering.

---

## ⚡ Execution Pipeline & Network Architecture

```
                                  ┌──► [Private MEV Relay (L1)] ──► Flashbots / Titan / Beaver
                                  │
[Trigger Hit] ──► [Pre-Signed Tx] ┼──► [Public RPC Racer (L1/L2)] ──► Fast Node Broadcast
                                  │
                                  └──► [Background Tx Monitor] ──► Receipt Poll ──► Auto-Speedup (RBF)
```

1. **Pre-Signed Transaction Cache:** Transactions are constructed and signed in memory *ahead of time*. When the trigger condition is met, Pulse doesn't spend time signing or calculating gas—it transmits the raw pre-signed bytes directly to open sockets.
2. **Chain-Aware Dual-Broadcasting:**
   - **Ethereum L1 (`chain_id == 1`):** Concurrently transmits to direct private MEV builders (`eth_sendPrivateTransaction` with EIP-191 authentication) and races public RPCs.
   - **EVM L2s (Robinhood `4663`, Base `8453`, Arbitrum `42161`):** Bypasses L1 builder relays and floods parallel public RPC endpoints simultaneously to guarantee inclusion by the sequencer.
3. **Keep-Alive Socket Warming:** Sockets are pre-warmed every 15–30 seconds with keep-alive pings, eliminating cold TCP/TLS handshake latency during the critical drop second.
4. **Replace-by-Fee (RBF) Auto-Speedup:** If a transaction remains unconfirmed after `speedup_threshold_ms`, Pulse calculates bumped gas fees (`+15%`), re-signs the same nonce, and re-broadcasts until confirmed or the hard ceiling is reached.

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
mev_builder_urls = []
rpc_timeout_ms = 3000

[drop]
target_contract = "0x0000000000000000000000000000000000000000"
mint_function = "mint(uint256)"
mint_value_wei = "0"
flip_function_signatures = [
    "isPublicSaleActive()",
    "saleIsActive()",
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

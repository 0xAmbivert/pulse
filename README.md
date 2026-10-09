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
  <img src="https://img.shields.io/badge/UI-Ratatui-purple.svg?style=flat-square" alt="TUI">
</p>

---

> 🔥 *Designed specifically for high-concurrency, deterministic execution during ruthless NFT drops and gas wars across **Ethereum L1** and **ALL EVM-compatible networks** (Robinhood Chain, Arbitrum, Base, Optimism, Polygon, BSC, etc.). Win the block, every time.*

## ✨ Core Superpowers

### 🛡️ Hardware-Grade Cryptographic Security (Zero Disk Persistence)
* **Direct RAM Isolation:** Zero keystores, zero unencrypted files. Private keys exist solely in `mlock`-pinned physical RAM and never touch disk, swap partitions, or page files.
* **No Passwords Required:** No persistent encrypted files to maintain, decrypt, or brute-force.
* **Poof! (`zeroize`):** Automatic memory scrubbing the exact millisecond the bot terminates safely or intercepts termination signals (`SIGTERM`, `SIGHUP`, `Ctrl-C`).
* **Blazing Pre-Signed Bursts:** Transactions are pre-signed in memory ahead of the drop and updated dynamically on base fee changes, enabling true $<1\text{ms}$ socket transmission upon trigger arrival.

### 🏎️ Parallel RPC Racing & Chain-Aware Relays
* **Multi-Route Racing:** Blasts signed transaction payloads concurrently across multiple HTTP/HTTPS RPC endpoints simultaneously.
* **Chain-Aware MEV Relaying:** On Ethereum L1, concurrently broadcasts to **MEV Block Builders** (Flashbots, Titan, BeaverBuild) and public RPC racers. On L2 networks (e.g. Robinhood Chain, Base, Arbitrum), automatically routes directly to parallel RPC racers without builder overhead.
* **Pre-Warmed Sockets & Ping HUD:** Keeps TCP and TLS handshakes warm with background keep-alive pings and renders real-time ping latencies on the dashboard.

### 🎯 Sniping Engines & Pre-Flight Checks
* **Advisory Pre-Flight Simulation:** Executes remote `eth_estimateGas` checks at boot to verify target contract and calldata validity without deadlocking critical execution loops.
* 🕵️ **Reactive Mempool Backrun:** Supports both WebSocket event streaming (`eth_subscribe` for `newPendingTransactions`) and HTTP pending block inspection. Stalks the owner's transaction and fires your mint in the exact same block!
* ⏱️ **Countdown Burst:** Calculates exact target Unix timestamps and bursts pre-signed transactions with sub-millisecond precision.
* 🔄 **State Poller:** Polls contract view functions (e.g. `isPublicSaleActive()`) to detect instant sale state flips.

### 🎛️ Terminal Command Center (TUI)
* An asynchronous Terminal UI built with `ratatui` providing a real-time HUD of live RPC latencies, dynamic network base fees, active wallet nonces, and transaction execution logs.

---

## 📦 Installation & Setup

### Prerequisites
You will need **Rust** and **Cargo** installed on your system to compile the bot.
```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### Clone & Build
```bash
# 1. Clone the repository
git clone https://github.com/0xAmbivert/pulse.git

# 2. Enter the directory
cd pulse

# 3. Build the highly-optimized release binary
cargo build --release
```

---

## 🛠️ Quick Start Guide

### 1️⃣ Launch Pulse

Forget manual config editing before every drop. Pulse features an interactive, ephemeral in-memory wizard. Just run:

```bash
cargo run --release
# or run the compiled binary directly:
./target/release/pulse
```

You will be greeted with the Main Menu:
```text
===============================
        🚀 Pulse 🚀          
===============================
1. 🟢 Start Sniping Engine (Ephemeral RAM Setup)
2. 🔍 Check Mint & Wallet Eligibility (OpenSea / MintGo / Contract)
3. ❌ Exit
👉 Choose an option: 
```

### 2️⃣ Check Mint & Wallet Eligibility (Option 2)
Selecting **Option 2** launches the automated drop intelligence & eligibility auditor:
* **Contract or OpenSea URL:** Paste any OpenSea collection link (e.g. `https://opensea.io/collection/housecraft`) or 42-char contract address.
* **Auto-Resolution:** Automatically extracts collection metadata, resolves the underlying contract address, and maps the chain network.
* **MintGo & SeaDrop Intelligence:** Queries MintGo drop intelligence for mint stages (presale, whitelist, public), start/end times, and prices.
* **On-Chain Wallet Audit:** Queries on-chain SeaDrop protocol state and contract mint stats against your specific wallet. Simulates execution via `eth_call` and `eth_estimateGas` to confirm if your wallet is eligible to mint right now, how many remaining tokens you can mint, or decodes the exact contract revert reason.
* **One-Click Load:** Offers to immediately load verified parameters into the RAM sniper engine!

### 3️⃣ Ephemeral RAM Setup (Option 1)
Selecting **Option 1** sets up your session directly in physical RAM:
1. **Private Key:** Paste your 64-character raw private key (hidden input via `rpassword`, never echoed or saved). It is decoded directly into an `mlock`-pinned memory buffer.
2. **Target Contract:** Enter the NFT contract address or paste an OpenSea URL (auto-resolved).
3. **RPC URL:** Press Enter to use the public default from `config.toml`, or enter a custom HTTP/HTTPS/WSS RPC.
4. **Mint Parameters:** Enter the mint function signature (auto-detected if SeaDrop), quantity, and value in Wei.
5. **Gas Settings:** Enter max fee ceiling or accept the default.
6. **Trigger Mode:** Select between:
   - `a` State Poller (Watches contract state flips)
   - `b` Countdown Timer (Target Unix timestamp)
   - `c` Mempool Backrun (Watches owner transaction)

### 4️⃣ Zero-Footprint Anti-Forensics
* **Zero Disk Persistence:** All keys, addresses, and drop parameters entered during setup live only in RAM for the active process.
* **On Exit:** Press `q` or `Esc` to quit. Pulse immediately zeroizes the memory buffer and frees allocated resources. Nothing is written to `config.toml` or any disk file.
* **Fresh Runs:** Pulse always starts clean with only public network presets, requiring fresh in-memory input every session.

---

## 🕹️ Operations & TUI Controls

* **Live Telemetry:** The UI updates asynchronously without blocking the main event loop, giving you real-time latency metrics and dynamic log readouts.
* **Replace-by-Fee Auto-Speedup:** If an in-flight transaction is not confirmed within the configured threshold, Pulse automatically bumps gas fees, re-signs, and re-broadcasts.
* **Bail Out:** Press `q` or `Esc` at any time to safely terminate the process. The bot catches the signal, cleanly restores the terminal, and immediately zeroizes memory to protect your keys. 🛑

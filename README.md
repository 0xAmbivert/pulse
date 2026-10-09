<h1 align="center">
  🚀 Pulse 🚀
</h1>

<p align="center">
  <strong>An industry-grade, ultra-low-latency NFT Sniper & Mint Bot engineered in pure Rust. 🦀</strong>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Rust-1.98+-orange.svg?style=flat-square" alt="Rust Version">
  <img src="https://img.shields.io/badge/EVM-Multi--Chain-blue.svg?style=flat-square" alt="EVM Multi-Chain">
  <img src="https://img.shields.io/badge/Security-Argon2id%2BAES256-brightgreen.svg?style=flat-square" alt="Security">
  <img src="https://img.shields.io/badge/UI-Ratatui-purple.svg?style=flat-square" alt="TUI">
</p>

---

> 🔥 *Designed specifically for high-concurrency, deterministic execution during ruthless NFT drops and gas wars across **Ethereum L1** and **ALL EVM-compatible networks** (Arbitrum, Optimism, Base, Polygon, BSC, etc.). Win the block, every time.*

## ✨ Core Superpowers

### 🛡️ Hardware-Grade Cryptographic Security
* **Direct RAM Isolation:** Zero disk persistence. Private keys live solely in `mlock`-pinned physical RAM and never touch disk, swap, or keystore files.
* **No Passwords Required:** No persistent encrypted files to maintain, decrypt, or brute-force.
* **Poof! (`zeroize`):** Automatic memory scrubbing the exact millisecond the bot terminates safely.
* **Blazing Signatures:** Fast local `secp256k1` signing. *Zero* network latency added for signatures!

### 🏎️ Parallel RPC Racing
* **Multi-Route Broadcasting:** Blasts your signed transaction payload concurrently across multiple HTTP/HTTPS RPCs simultaneously!
* **Dark Forest Mastery:** Native support for **MEV Block Builder** direct relays (Flashbots, Titan) to bypass the public mempool, stop frontrunners, and guarantee top-of-block inclusion. 🥷

### 🎯 Sniping Engines & Pre-Flight Checks
* **RPC Simulation & Verification:** Automatically executes remote `eth_estimateGas` pre-flight checks to prevent failed transactions and revert penalties, dynamically handling pending mempool backruns.
* 🕵️ **Mempool Backrun:** Stalks the pending mempool for the owner's transaction and fires your mint right behind it!
* ⏱️ **Countdown Burst:** Calculates exact target Unix timestamps and bursts transactions with sub-millisecond zero-latency precision.
* 🔄 **State Poller:** Hammers contract view functions dynamically on every new block header.

### 🎛️ Terminal Command Center (TUI)
* A gorgeous, asynchronous Terminal UI built with `ratatui` giving you a real-time HUD of latency pinging, gas tracking, active workers, and transaction events.

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

Forget manually typing CLI flags or editing TOML files. Pulse features a seamless interactive **Main Menu Wizard**. Just run:

```bash
cargo run --release
```

You will be greeted with the Main Menu:
```text
===============================
        🚀 Pulse 🚀          
===============================
1. 🟢 Start Sniping Engine (Direct RAM Key)
2. ⚙️  Setup / Edit Config
3. ❌ Exit
👉 Choose an option: 
```

### 2️⃣ Configure the Drop
* **Press 2** to launch the interactive Configuration Wizard.
* Configure your target Chain ID, RPC URL, NFT Contract Address, and maximum Gas limits.
* Automatically saves your parameters to `config.toml`.

### 3️⃣ Arm the Snipers (RAM Mode)
* **Press 1** to start the engine.
* Paste your raw private key (hidden input via `rpassword`, never echoed or logged).
* The key is parsed directly into `mlock`-pinned memory and never written to disk.
* You will instantly drop into the **Live Terminal Dashboard**! 🎛️
* When you press `q` or `Esc`, the key is immediately zeroized and purged from RAM. 🛑

---

## 🕹️ Operations & TUI Controls

* **Live Telemetry:** The UI updates asynchronously without blocking the main event loop, giving you real-time latency metrics and dynamic log readouts. 
* **Bail Out:** Press `q` or `Esc` at any time to safely terminate the process. The bot will catch the signal, cleanly lock, and immediately zeroize memory to protect your keys. 🛑

<h1 align="center">
  🚀 EVM Apex Mint Bot 🚀
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
* **Fort Knox at Rest:** Uses standard-defining `Argon2id` + `AES-256-GCM` encrypted keystores.
* **Pinned RAM (`mlock`):** Prevents the OS from *ever* writing your private keys to disk or swap partitions.
* **Poof! (`zeroize`):** Automatic memory scrubbing the exact millisecond the bot terminates.
* **Blazing Signatures:** Sub-microsecond local `secp256k1` signing. *Zero* network latency added for signatures!

### 🏎️ Parallel RPC Racing
* **Multi-Route Broadcasting:** Blasts your signed transaction payload concurrently across multiple WebSocket/HTTP RPCs simultaneously!
* **Dark Forest Mastery:** Native support for **MEV Block Builder** direct relays (Flashbots, Titan, BeaverBuild) to bypass the public mempool, stop frontrunners, and guarantee top-of-block inclusion. 🥷

### 🎯 Sub-Millisecond Sniping Engines
* 🕵️ **Mempool Backrun:** Stalks the pending mempool for the owner's transaction (e.g. `setPublicSaleActive()`) and fires your mint *in the exact same block* right behind it!
* ⏱️ **Countdown Burst:** Calculates exact target Unix timestamps and bursts pre-signed transactions with sub-millisecond precision.
* 🔄 **State Poller:** Hammers contract view functions dynamically on every new block header.

### 🎛️ Terminal Command Center (TUI)
* A gorgeous, asynchronous Terminal UI built with `ratatui` giving you a real-time HUD of latency pinging, gas tracking, active workers, and transaction events.

---

## 🛠️ Quick Start Guide

### 1️⃣ Configure the Beast (`config.toml`)

Before launching, you must define the `config.toml` file in the root directory. Configure it for your battlefield of choice (Ethereum Mainnet, Arbitrum, Base, etc.):

```toml
[chain]
# The battleground: 1 for ETH Mainnet, 42161 for Arbitrum, 8453 for Base!
chain_id = 1
name = "Ethereum Mainnet"

# 🚀 Your premium, high-octane RPC endpoints
rpc_urls = [
    "https://eth.llamarpc.com",
    "https://eth-mainnet.g.alchemy.com/v2/YOUR_API_KEY",
]

# 🥷 The Dark Forest (MEV Relays for Ethereum L1 to bypass the mempool)
# Leave empty `[]` for L2 networks like Base or Arbitrum.
mev_builder_urls = [
    "https://relay.flashbots.net",
    "https://rpc.titanbuilder.xyz",
    "https://rpc.beaverbuild.org"
]
rpc_timeout_ms = 1500

[drop]
target_contract = "0xYOUR_NFT_CONTRACT_ADDRESS"
mint_function = "mint(uint256)"
mint_value_wei = "0"

# 🎯 Example of targeting a precise timestamp launch
target_timestamp = 1750000000 
monitor_owner_address = "0xOWNER_ADDRESS"
flip_function_signatures = ["setPublicSaleActive(bool)", "unpause()"]

[gas]
# ⛽ Fine-tune these based on the specific chain's gas mechanics
max_fee_gwei = 50.0
max_priority_fee_gwei = 3.0
gas_limit = 150000

# 🏎️ Auto-Speedup Mechanism
auto_speedup = true
speedup_threshold_ms = 12000
speedup_bump_percent = 15
hard_gas_ceiling_gwei = 150.0

[wallets]
# 💼 Your loaded weapons
keystore_paths = ["./keystores/wallet_01.json"]

[alerts]
# 🔔 Ding! Get notified when you win
discord_webhook = ""
```

### 2️⃣ Forge a Secure Wallet

Never put raw private keys into plaintext files! Generate a military-grade AES-encrypted keystore directly from the CLI:

```bash
cargo run -- --generate-wallet
```
1. You will be prompted to set a **Master Passphrase**. 🤫
2. The bot generates a completely random secure Ethereum wallet and stores it in `./keystores/<address>.json`.
3. Copy the outputted public address and **fund it** with the native gas token of your target chain! 💸
4. Update your `config.toml` so the `keystore_paths` array points to your shiny new `.json` file.

### 3️⃣ Launch the Bot

Once your wallet is funded and config is locked in, wake the bot up:

```bash
cargo run --release
```
* The bot will ask for your Master Passphrase to securely decrypt the keys into RAM. 🧠
* You will instantly drop into the **Live Terminal Dashboard**! 🎛️
* The Snipers will arm themselves and await the drop trigger. When the condition hits, the bot signs locally and parallel-broadcasts to all configured RPC and MEV endpoints in a fraction of a millisecond! 💥

---

## 🕹️ Operations & TUI Controls

* **Live Telemetry:** The UI updates asynchronously without blocking the main event loop, giving you real-time latency metrics and dynamic log readouts. 
* **Bail Out:** Press `q` or `Esc` at any time to safely terminate the process. The bot will catch the signal, cleanly lock, and immediately zeroize memory to protect your keys. 🛑

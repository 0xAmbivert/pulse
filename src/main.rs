#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_imports)]

mod alerts;
mod config;
mod crypto;
mod gas;
mod network;
mod simulation;
mod sniper;
mod ui;
mod wallet;

use clap::Parser;
use crossterm::{
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;
use rand::RngCore;

use crate::config::AppConfig;
use crate::sniper::SnipeTrigger;
use crate::ui::{draw_ui, AppEvent, DashboardState, EventHandler};
use crate::crypto::{ProtectedKey, encrypt_key_to_file, get_address_from_protected};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Path to configuration file
    #[arg(short, long, value_name = "FILE", default_value = "config.toml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli = Cli::parse();
    
    // Create default config silently if it doesn't exist so load won't fail
    if !cli.config.exists() {
        AppConfig::default().save_to_file(&cli.config)?;
    }

    loop {
        println!("\n===============================");
        println!("        🚀 Pulse 🚀          ");
        println!("===============================");
        println!("1. 🟢 Start Sniping Engine");
        println!("2. ➕ Generate New Wallet");
        println!("3. 📥 Import Private Key");
        println!("4. ⚙️  Setup / Edit Config");
        println!("5. ❌ Exit");
        print!("👉 Choose an option: ");
        io::stdout().flush()?;
        
        let mut choice = String::new();
        io::stdin().read_line(&mut choice)?;
        
        match choice.trim() {
            "1" => {
                println!("Booting Sniping Engine...");
                break;
            }
            "2" => {
                println!("\n--- Generate Wallet ---");
                print!("Enter new master passphrase: ");
                io::stdout().flush()?;
                let mut pass = String::new();
                io::stdin().read_line(&mut pass)?;
                
                let mut key_bytes = [0u8; 32];
                rand::rngs::OsRng.fill_bytes(&mut key_bytes);
                let key = ProtectedKey::new(key_bytes);
                let address = get_address_from_protected(&key)?;
                
                std::fs::create_dir_all("./keystores").unwrap_or_default();
                let path = Path::new("./keystores").join(format!("{}.json", address));
                encrypt_key_to_file(&key, &address, pass.trim(), &path)?;
                
                println!("✅ Successfully generated and encrypted wallet!");
                println!("Public Address: {}", address);
                println!("Keystore Path: {}", path.display());
            }
            "3" => {
                println!("\n--- Import Wallet ---");
                print!("Paste your raw private key (Hex): ");
                io::stdout().flush()?;
                let mut pk_hex = String::new();
                io::stdin().read_line(&mut pk_hex)?;
                let pk_hex = pk_hex.trim().trim_start_matches("0x");
                
                if pk_hex.len() != 64 {
                    println!("❌ Invalid private key length. Must be 64 hex characters.");
                    continue;
                }
                
                let mut key_bytes = [0u8; 32];
                match hex::decode_to_slice(pk_hex, &mut key_bytes) {
                    Ok(_) => {
                        print!("Enter Master Passphrase to encrypt this key: ");
                        io::stdout().flush()?;
                        let mut pass = String::new();
                        io::stdin().read_line(&mut pass)?;
                        
                        let key = ProtectedKey::new(key_bytes);
                        let address = get_address_from_protected(&key)?;
                        std::fs::create_dir_all("./keystores").unwrap_or_default();
                        let path = Path::new("./keystores").join(format!("{}.json", address));
                        encrypt_key_to_file(&key, &address, pass.trim(), &path)?;
                        
                        println!("✅ Successfully imported and encrypted wallet!");
                        println!("Public Address: {}", address);
                    }
                    Err(_) => println!("❌ Invalid hex characters in private key."),
                }
                
                // Clear the hex from memory immediately
                pk_hex.to_string().clear(); 
            }
            "4" => {
                println!("\n--- Configuration Wizard ---");
                let mut config = AppConfig::default();
                
                print!("🔗 Enter Chain ID (e.g., 1 for ETH, 8453 for Base): ");
                io::stdout().flush()?;
                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
                config.chain.chain_id = input.trim().parse().unwrap_or(1);

                print!("🌐 Enter RPC URL (WebSocket or HTTP): ");
                io::stdout().flush()?;
                input.clear();
                io::stdin().read_line(&mut input)?;
                if !input.trim().is_empty() {
                    config.chain.rpc_urls = vec![input.trim().to_string()];
                }

                print!("🎯 Enter Target NFT Contract Address: ");
                io::stdout().flush()?;
                input.clear();
                io::stdin().read_line(&mut input)?;
                if !input.trim().is_empty() {
                    config.drop.target_contract = input.trim().to_string();
                }

                print!("💰 Enter Max Gas Fee in Gwei (e.g., 50.0): ");
                io::stdout().flush()?;
                input.clear();
                io::stdin().read_line(&mut input)?;
                config.gas.max_fee_gwei = input.trim().parse().unwrap_or(50.0);
                
                config.save_to_file(&cli.config)?;
                println!("✅ Config saved to {}!", cli.config.display());
            }
            "5" => {
                println!("Goodbye!");
                return Ok(());
            }
            _ => println!("❌ Invalid option. Try again."),
        }
    }

    let config = AppConfig::load_from_file(&cli.config)?;

    // Setup Engine Components
    let rpc_racer = Arc::new(crate::network::RpcRacer::new(&config.chain.rpc_urls, config.chain.rpc_timeout_ms));
    let _gas_engine = crate::gas::GasEngine::new(config.gas.hard_gas_ceiling_gwei, config.gas.max_priority_fee_gwei, config.gas.speedup_bump_percent);
    
    // Decrypt wallets securely into memory
    print!("\n🔑 Enter master passphrase to unlock configured wallets: ");
    io::stdout().flush()?;
    let mut password = String::new();
    io::stdin().read_line(&mut password)?;
    let password = password.trim();

    let mut workers = Vec::new();
    for path in &config.wallets.keystore_paths {
        let p = Path::new(path);
        if p.exists() {
            match crate::crypto::decrypt_key_from_file(p, password) {
                Ok((key, addr)) => {
                    if let Ok(worker) = crate::wallet::WalletWorker::new(key, 0) {
                        workers.push(Arc::new(worker));
                        println!("🔓 Unlocked wallet: {}", addr);
                    }
                }
                Err(_) => println!("❌ Failed to decrypt wallet {}. Wrong password?", p.display()),
            }
        } else {
            println!("⚠️ Configured keystore not found: {}", path);
        }
    }

    if workers.is_empty() {
        println!("❌ No active wallets loaded. Please generate or import a wallet via the wizard first.");
        return Ok(());
    }

    // Setup Terminal UI
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = DashboardState::new(config.drop.target_contract.clone());
    state.active_wallets = workers.len();
    state.current_base_fee = config.gas.max_fee_gwei; // Default baseline

    let mut events = EventHandler::new(250);
    let (trigger_tx, mut trigger_rx) = mpsc::channel::<SnipeTrigger>(10);

    // Optional: We can spawn the MempoolScanner here and pass `trigger_tx.clone()`
    let target_contract = config.drop.target_contract.clone();
    let rpc_racer_clone = rpc_racer.clone();

    // Main Event Loop
    while state.is_running {
        terminal.draw(|f| draw_ui(f, &state))?;

        tokio::select! {
            Some(event) = events.next() => {
                match event {
                    AppEvent::Input(key) => state.handle_key(key.code),
                    AppEvent::Tick => {} 
                }
            }
            Some(trigger) = trigger_rx.recv() => {
                let msg = match trigger {
                    SnipeTrigger::CountdownReached { target_unix } => format!("🔥 Trigger: Countdown reached {}", target_unix),
                    SnipeTrigger::MempoolDetected { owner_tx_hash, method } => format!("🔥 Trigger: Mempool flip {} via {}", owner_tx_hash, method),
                    SnipeTrigger::StateFlipDetected { new_state } => format!("🔥 Trigger: State flip to {}", new_state),
                    SnipeTrigger::BlockReached { target_block } => format!("🔥 Trigger: Target block {} reached", target_block),
                };
                state.add_log(msg.clone());

                // FIRE MINT TRANSACTION ACROSS ALL WALLETS
                for worker in &workers {
                    // Extract calldata config
                    let calldata_hex = config.drop.custom_calldata_hex.clone().unwrap_or_else(|| "0x00".to_string());
                    let calldata = hex::decode(calldata_hex.trim_start_matches("0x")).unwrap_or_default();
                    let value = u128::from_str_radix(&config.drop.mint_value_wei, 10).unwrap_or(0);
                    
                    let max_fee_wei = crate::gas::gwei_to_wei(config.gas.max_fee_gwei);
                    let max_priority_fee_wei = crate::gas::gwei_to_wei(config.gas.max_priority_fee_gwei);

                    match worker.build_and_sign_eip1559(
                        config.chain.chain_id,
                        &target_contract,
                        &calldata,
                        value,
                        config.gas.gas_limit,
                        max_fee_wei,
                        max_priority_fee_wei,
                        None,
                    ) {
                        Ok((raw_tx, nonce)) => {
                            state.add_log(format!("🚀 Signed tx for {} (Nonce: {})", worker.address, nonce));
                            
                            // Broadcast concurrently via RPC Racer
                            let racer = rpc_racer_clone.clone();
                            let raw_tx_clone = raw_tx.clone();
                            
                            tokio::spawn(async move {
                                let outcomes = racer.race_broadcast_raw_tx(&raw_tx_clone).await;
                                for outcome in outcomes {
                                    if outcome.success {
                                        // A real bot would log back to the UI channel here
                                    }
                                }
                            });
                        }
                        Err(e) => {
                            state.add_log(format!("❌ Signing failed for {}: {}", worker.address, e));
                        }
                    }
                }
            }
        }
    }

    // Restore Terminal
    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;
    println!("Bot shutdown gracefully.");

    Ok(())
}

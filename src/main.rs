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

    /// Generate a new AES-256-GCM encrypted keystore file
    #[arg(long)]
    generate_wallet: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli = Cli::parse();
    
    // Interactive Setup Wizard if config doesn't exist
    if !cli.config.exists() && !cli.generate_wallet {
        println!("✨ No config file found. Let's set up your bot!");
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
        config.chain.rpc_urls = vec![input.trim().to_string()];

        print!("🎯 Enter Target NFT Contract Address: ");
        io::stdout().flush()?;
        input.clear();
        io::stdin().read_line(&mut input)?;
        config.drop.target_contract = input.trim().to_string();

        print!("💰 Enter Max Gas Fee in Gwei (e.g., 50.0): ");
        io::stdout().flush()?;
        input.clear();
        io::stdin().read_line(&mut input)?;
        config.gas.max_fee_gwei = input.trim().parse().unwrap_or(50.0);
        
        config.save_to_file(&cli.config)?;
        println!("✅ Config saved to {}!\n", cli.config.display());
    }

    if cli.generate_wallet {
        println!("Generating new secure wallet...");
        print!("Enter new master passphrase: ");
        io::stdout().flush()?;
        let mut pass = String::new();
        io::stdin().read_line(&mut pass)?;
        let pass = pass.trim();

        let mut key_bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut key_bytes);
        let key = ProtectedKey::new(key_bytes);
        let address = get_address_from_protected(&key)?;
        
        let path = Path::new("./keystores").join(format!("{}.json", address));
        encrypt_key_to_file(&key, &address, pass, &path)?;
        
        println!("✅ Successfully generated and encrypted wallet!");
        println!("Public Address: {}", address);
        println!("Keystore Path: {}", path.display());
        return Ok(());
    }

    let config = AppConfig::load_from_file(&cli.config)?;

    // Setup Engine Components
    let rpc_racer = Arc::new(crate::network::RpcRacer::new(&config.chain.rpc_urls, config.chain.rpc_timeout_ms));
    let _gas_engine = crate::gas::GasEngine::new(config.gas.hard_gas_ceiling_gwei, config.gas.max_priority_fee_gwei, config.gas.speedup_bump_percent);
    
    // Decrypt wallets securely into memory
    print!("🔑 Enter master passphrase to unlock configured wallets: ");
    io::stdout().flush()?;
    let mut password = String::new();
    io::stdin().read_line(&mut password)?;
    let password = password.trim();

    let mut workers = Vec::new();
    for path in &config.wallets.keystore_paths {
        let p = Path::new(path);
        if p.exists() {
            let (key, addr) = crate::crypto::decrypt_key_from_file(p, password)?;
            let worker = crate::wallet::WalletWorker::new(key, 0)?; // Initializing with 0, will resync naturally
            workers.push(Arc::new(worker));
            println!("🔓 Unlocked wallet: {}", addr);
        } else {
            println!("⚠️ Configured keystore not found: {}", path);
        }
    }

    if workers.is_empty() {
        println!("❌ No active wallets loaded. Please generate a wallet or check config.toml");
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

use std::io::{self, IsTerminal, Write};

use crate::config::AppConfig;
use crate::crypto::ProtectedKey;

pub enum MenuAction {
    Start {
        key: ProtectedKey,
        session_config: Box<AppConfig>,
    },
    Exit,
}

/// Displays the interactive CLI menu and returns the ephemeral session configuration.
pub fn run_interactive_menu(base_config: &AppConfig) -> Result<MenuAction, Box<dyn std::error::Error + Send + Sync>> {
    loop {
        println!("\n===============================");
        println!("        🚀 Pulse 🚀          ");
        println!("===============================");
        println!("1. 🟢 Start Sniping Engine (Ephemeral RAM Setup)");
        println!("2. ❌ Exit");
        print!("👉 Choose an option: ");
        io::stdout().flush()?;

        let mut choice = String::new();
        if io::stdin().read_line(&mut choice)? == 0 {
            return Ok(MenuAction::Exit);
        }

        match choice.trim() {
            "1" => {
                println!("\n--- ⚡ Ephemeral RAM Configuration (Zero Disk Saves) ---");
                println!("🔒 Everything entered here lives only in RAM for this active session and is wiped on exit.\n");

                // 1. Private Key (RAM only, hidden input via rpassword when interactive, stdin when piped)
                let pk_input = if io::stdin().is_terminal() {
                    match rpassword::prompt_password("1️⃣  Paste raw private key (Hex): ") {
                        Ok(p) => p,
                        Err(e) => {
                            println!("❌ Failed to read private key: {}", e);
                            continue;
                        }
                    }
                } else {
                    print!("1️⃣  Paste raw private key (Hex): ");
                    io::stdout().flush()?;
                    let mut line = String::new();
                    if io::stdin().read_line(&mut line)? == 0 {
                        return Ok(MenuAction::Exit);
                    }
                    line.trim().to_string()
                };
                let pk_guard = zeroize::Zeroizing::new(pk_input);
                let clean_hex = pk_guard.as_str().trim().trim_start_matches("0x");

                if clean_hex.len() != 64 {
                    println!("❌ Invalid private key length. Must be 64 hex characters.");
                    continue;
                }

                let key = match ProtectedKey::from_hex(clean_hex) {
                    Ok(k) => k,
                    Err(e) => {
                        println!("❌ Invalid hex key: {}", e);
                        continue;
                    }
                };

                // 2. Target Contract Address
                print!("2️⃣  Enter Target NFT Contract Address: ");
                io::stdout().flush()?;
                let mut target = String::new();
                io::stdin().read_line(&mut target)?;
                let target_addr = target.trim().to_string();
                if target_addr.is_empty() {
                    println!("❌ Target contract address cannot be empty.");
                    continue;
                }

                // 3. RPC URL (Optional: Enter to keep public default from config)
                let default_rpc = base_config.chain.rpc_urls.first().cloned().unwrap_or_else(|| "https://rpc.mainnet.chain.robinhood.com".to_string());
                print!("3️⃣  Enter RPC URL (Press Enter for public default '{}'): ", default_rpc);
                io::stdout().flush()?;
                let mut rpc_input = String::new();
                io::stdin().read_line(&mut rpc_input)?;
                let rpc_url = if rpc_input.trim().is_empty() {
                    default_rpc
                } else {
                    rpc_input.trim().to_string()
                };

                // 4. Mint Function Signature (Optional: Enter for default)
                print!("4️⃣  Enter Mint Function (Press Enter for '{}'): ", base_config.drop.mint_function);
                io::stdout().flush()?;
                let mut mint_sig = String::new();
                io::stdin().read_line(&mut mint_sig)?;
                let mint_function = if mint_sig.trim().is_empty() {
                    base_config.drop.mint_function.clone()
                } else {
                    mint_sig.trim().to_string()
                };

                // 5. Mint Value in Wei (Optional: Enter for default)
                print!("5️⃣  Enter Mint Value in Wei (Press Enter for '{}'): ", base_config.drop.mint_value_wei);
                io::stdout().flush()?;
                let mut val_input = String::new();
                io::stdin().read_line(&mut val_input)?;
                let mint_value_wei = if val_input.trim().is_empty() {
                    base_config.drop.mint_value_wei.clone()
                } else {
                    val_input.trim().to_string()
                };

                // 6. Max Gas Fee in Gwei (Optional: Enter for default)
                print!("6️⃣  Enter Max Gas Fee in Gwei (Press Enter for '{:.1}'): ", base_config.gas.max_fee_gwei);
                io::stdout().flush()?;
                let mut gas_input = String::new();
                io::stdin().read_line(&mut gas_input)?;
                let max_gas_fee = if gas_input.trim().is_empty() {
                    base_config.gas.max_fee_gwei
                } else {
                    gas_input.trim().parse().unwrap_or(base_config.gas.max_fee_gwei)
                };

                // 7. Snipe Trigger Mode Selection
                println!("\n🎯 Select Sniping Trigger Mode:");
                println!("   a) Instant / State Poller (Watches when contract sale flips active)");
                println!("   b) Countdown Timer (Target Unix timestamp)");
                println!("   c) Mempool Backrun (Watches owner transaction)");
                print!("👉 Choose trigger mode [a/b/c] (default 'a'): ");
                io::stdout().flush()?;
                let mut trigger_choice = String::new();
                io::stdin().read_line(&mut trigger_choice)?;

                let mut target_timestamp = None;
                let mut monitor_owner_address = None;

                match trigger_choice.trim().to_lowercase().as_str() {
                    "b" => {
                        print!("   Enter Target Unix Timestamp: ");
                        io::stdout().flush()?;
                        let mut ts_str = String::new();
                        io::stdin().read_line(&mut ts_str)?;
                        target_timestamp = ts_str.trim().parse::<u64>().ok();
                    }
                    "c" => {
                        print!("   Enter Owner Address to Monitor: ");
                        io::stdout().flush()?;
                        let mut owner_str = String::new();
                        io::stdin().read_line(&mut owner_str)?;
                        if !owner_str.trim().is_empty() {
                            monitor_owner_address = Some(owner_str.trim().to_string());
                        }
                    }
                    _ => {}
                }

                // Construct session configuration strictly in memory (NEVER saved to file)
                let mut session_config = base_config.clone();
                session_config.chain.rpc_urls = vec![rpc_url];
                session_config.drop.target_contract = target_addr;
                session_config.drop.mint_function = mint_function;
                session_config.drop.mint_value_wei = mint_value_wei;
                session_config.drop.target_timestamp = target_timestamp;
                session_config.drop.monitor_owner_address = monitor_owner_address;
                session_config.gas.max_fee_gwei = max_gas_fee;

                println!("\n🔒 Session parameters locked into memory buffer. Booting engine...");
                return Ok(MenuAction::Start { key, session_config: Box::new(session_config) });
            }
            "2" => {
                println!("Goodbye!");
                return Ok(MenuAction::Exit);
            }
            _ => println!("❌ Invalid option. Try again."),
        }
    }
}

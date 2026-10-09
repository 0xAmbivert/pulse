use std::io::{self, Write};
use std::path::Path;

use crate::config::AppConfig;
use crate::crypto::ProtectedKey;

pub enum MenuAction {
    Start(ProtectedKey),
    Exit,
}

/// Displays the interactive CLI menu and returns the selected action.
pub fn run_interactive_menu(config_path: &Path) -> Result<MenuAction, Box<dyn std::error::Error + Send + Sync>> {
    loop {
        println!("\n===============================");
        println!("        🚀 Pulse 🚀          ");
        println!("===============================");
        println!("1. 🟢 Start Sniping Engine (Direct RAM Key)");
        println!("2. ⚙️  Setup / Edit Config");
        println!("3. ❌ Exit");
        print!("👉 Choose an option: ");
        io::stdout().flush()?;

        let mut choice = String::new();
        io::stdin().read_line(&mut choice)?;

        match choice.trim() {
            "1" => {
                println!("\n--- ⚡ Direct RAM Mode ---");
                println!("🔒 The private key is held strictly in mlock-pinned RAM and NEVER written to disk.");
                let pk_input = match rpassword::prompt_password("Paste raw private key (Hex): ") {
                    Ok(p) => p,
                    Err(e) => {
                        println!("❌ Failed to read private key: {}", e);
                        continue;
                    }
                };
                let pk_guard = zeroize::Zeroizing::new(pk_input);
                let clean_hex = pk_guard.as_str().trim().trim_start_matches("0x");

                if clean_hex.len() != 64 {
                    println!("❌ Invalid private key length. Must be 64 hex characters.");
                    continue;
                }

                match ProtectedKey::from_hex(clean_hex) {
                    Ok(key) => {
                        println!("🔒 Private key verified & pinned in RAM (zero disk storage).");
                        return Ok(MenuAction::Start(key));
                    }
                    Err(e) => {
                        println!("❌ Invalid hex key: {}", e);
                        continue;
                    }
                }
            }
            "2" => {
                println!("\n--- Configuration Wizard ---");
                let mut config = AppConfig::default();

                print!("🔗 Enter Chain ID (e.g., 1 for ETH, 4663 for Robinhood, 8453 for Base): ");
                io::stdout().flush()?;
                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
                config.chain.chain_id = input.trim().parse().unwrap_or(1);

                print!("🌐 Enter RPC URL (HTTP or HTTPS): ");
                io::stdout().flush()?;
                input.clear();
                io::stdin().read_line(&mut input)?;
                let trimmed = input.trim();
                if !trimmed.is_empty() {
                    if trimmed.starts_with("ws://") || trimmed.starts_with("wss://") {
                        println!("⚠️ Notice: RpcRacer requires HTTP/HTTPS endpoints. Please configure an http:// or https:// URL.");
                    }
                    config.chain.rpc_urls = vec![trimmed.to_string()];
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

                config.save_to_file(config_path)?;
                println!("✅ Config saved to {}!", config_path.display());
            }
            "3" => {
                println!("Goodbye!");
                return Ok(MenuAction::Exit);
            }
            _ => println!("❌ Invalid option. Try again."),
        }
    }
}

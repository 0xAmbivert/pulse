use std::io::{self, Write};
use std::path::Path;
use rand::RngCore;

use crate::config::AppConfig;
use crate::crypto::{encrypt_key_to_file, get_address_from_protected, ProtectedKey};

/// Displays the interactive CLI menu and returns `Ok(true)` if user chose to start the engine, or `Ok(false)` to exit.
pub fn run_interactive_menu(config_path: &Path) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
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
                return Ok(true);
            }
            "2" => {
                println!("\n--- Generate Wallet ---");
                let pass_input = match rpassword::prompt_password("Enter new master passphrase (min 10 chars): ") {
                    Ok(p) => p,
                    Err(e) => {
                        println!("❌ Failed to read passphrase: {}", e);
                        continue;
                    }
                };
                let pass = zeroize::Zeroizing::new(pass_input);
                if pass.as_str().trim().len() < 10 {
                    println!("❌ Passphrase must be at least 10 characters.");
                    continue;
                }

                let mut key = ProtectedKey::empty();
                rand::rngs::OsRng.fill_bytes(key.as_mut_bytes());
                let address = get_address_from_protected(&key)?;

                let path = Path::new("./keystores").join(format!("{}.json", address));
                encrypt_key_to_file(&key, &address, pass.as_str().trim(), &path)?;

                if let Ok(mut config) = AppConfig::load_from_file(config_path) {
                    let path_str = path.to_string_lossy().to_string();
                    if !config.wallets.keystore_paths.contains(&path_str) {
                        config.wallets.keystore_paths.push(path_str);
                        let _ = config.save_to_file(config_path);
                    }
                }

                println!("✅ Successfully generated and encrypted wallet!");
                println!("Public Address: {}", address);
                println!("Keystore Path: {}", path.display());
            }
            "3" => {
                println!("\n--- Import Wallet ---");
                let pk_input = match rpassword::prompt_password("Paste your raw private key (Hex): ") {
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

                let mut key = ProtectedKey::empty();
                match hex::decode_to_slice(clean_hex, key.as_mut_bytes()) {
                    Ok(_) => {
                        let pass_input = match rpassword::prompt_password("Enter Master Passphrase to encrypt this key (min 10 chars): ") {
                            Ok(p) => p,
                            Err(e) => {
                                println!("❌ Failed to read passphrase: {}", e);
                                continue;
                            }
                        };
                        let pass = zeroize::Zeroizing::new(pass_input);
                        if pass.as_str().trim().len() < 10 {
                            println!("❌ Passphrase must be at least 10 characters.");
                            continue;
                        }

                        let address = get_address_from_protected(&key)?;
                        let path = Path::new("./keystores").join(format!("{}.json", address));
                        encrypt_key_to_file(&key, &address, pass.as_str().trim(), &path)?;

                        if let Ok(mut config) = AppConfig::load_from_file(config_path) {
                            let path_str = path.to_string_lossy().to_string();
                            if !config.wallets.keystore_paths.contains(&path_str) {
                                config.wallets.keystore_paths.push(path_str);
                                let _ = config.save_to_file(config_path);
                            }
                        }

                        println!("✅ Successfully imported and encrypted wallet!");
                        println!("Public Address: {}", address);
                    }
                    Err(_) => println!("❌ Invalid hex characters in private key."),
                }
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

                config.save_to_file(config_path)?;
                println!("✅ Config saved to {}!", config_path.display());
            }
            "5" => {
                println!("Goodbye!");
                return Ok(false);
            }
            _ => println!("❌ Invalid option. Try again."),
        }
    }
}

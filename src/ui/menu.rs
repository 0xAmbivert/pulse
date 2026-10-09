use std::io::{self, Write};
use std::path::Path;
use rand::RngCore;

use crate::config::AppConfig;
use crate::crypto::{encrypt_key_to_file, get_address_from_protected, ProtectedKey};

pub enum MenuAction {
    StartKeystore,
    StartEphemeral(ProtectedKey),
    Exit,
}

/// Displays the interactive CLI menu and returns the selected action.
pub fn run_interactive_menu(config_path: &Path) -> Result<MenuAction, Box<dyn std::error::Error + Send + Sync>> {
    loop {
        println!("\n===============================");
        println!("        🚀 Pulse 🚀          ");
        println!("===============================");
        println!("1. 🟢 Start Engine (Saved Keystores)");
        println!("2. ⚡ Ephemeral RAM Mode (Paste Key, zero disk saves)");
        println!("3. ➕ Generate Encrypted Keystore");
        println!("4. 📥 Import to Encrypted Keystore");
        println!("5. ⚙️  Setup / Edit Config");
        println!("6. ❌ Exit");
        print!("👉 Choose an option: ");
        io::stdout().flush()?;

        let mut choice = String::new();
        io::stdin().read_line(&mut choice)?;

        match choice.trim() {
            "1" => {
                println!("Booting Sniping Engine from saved keystores...");
                return Ok(MenuAction::StartKeystore);
            }
            "2" => {
                println!("\n--- ⚡ Ephemeral RAM-Only Mode ---");
                println!("ℹ️ The private key lives strictly in mlock-pinned RAM and is NEVER saved to disk.");
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
                        println!("🔒 Private key loaded directly into memory buffer (zero disk writes).");
                        return Ok(MenuAction::StartEphemeral(key));
                    }
                    Err(e) => {
                        println!("❌ Failed to parse private key: {}", e);
                        continue;
                    }
                }
            }
            "3" => {
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
            "4" => {
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
            "5" => {
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
            "6" => {
                println!("Goodbye!");
                return Ok(MenuAction::Exit);
            }
            _ => println!("❌ Invalid option. Try again."),
        }
    }
}

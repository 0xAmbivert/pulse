use std::io::{self, IsTerminal, Write};

use crate::config::AppConfig;
use crate::crypto::ProtectedKey;
use crate::eligibility::checker::EligibilityChecker;
use crate::eligibility::opensea::OpenSeaClient;

pub enum MenuAction {
    Start {
        key: ProtectedKey,
        session_config: Box<AppConfig>,
    },
    Exit,
}

/// Displays the interactive CLI menu and returns the ephemeral session configuration.
pub async fn run_interactive_menu(base_config: &AppConfig) -> Result<MenuAction, Box<dyn std::error::Error + Send + Sync>> {
    loop {
        println!("\n===============================");
        println!("        🚀 Pulse 🚀          ");
        println!("===============================");
        println!("1. 🟢 Start Sniping Engine (Ephemeral RAM Setup)");
        println!("2. 🔍 Check Mint & Wallet Eligibility (OpenSea / MintGo / Contract)");
        println!("3. ❌ Exit");
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

                // 2. Target Contract Address or OpenSea URL
                print!("2️⃣  Enter Target NFT Contract Address or OpenSea URL: ");
                io::stdout().flush()?;
                let mut target = String::new();
                io::stdin().read_line(&mut target)?;
                let target_input = target.trim().to_string();
                if target_input.is_empty() {
                    println!("❌ Target contract address cannot be empty.");
                    continue;
                }

                // If user entered an OpenSea URL, resolve it automatically!
                let (target_addr, mut detected_mint_func, mut detected_price_wei) = if let Some(slug) = OpenSeaClient::extract_slug(&target_input) {
                    println!("🔎 Resolving OpenSea collection slug: '{}'...", slug);
                    let opensea = OpenSeaClient::new();
                    match opensea.fetch_collection_by_slug(&slug).await {
                        Ok(resolved) => {
                            println!("✅ Resolved OpenSea drop: '{}' -> {}", resolved.collection_name, resolved.contract_address);
                            (resolved.contract_address, None, None)
                        }
                        Err(e) => {
                            println!("⚠️ OpenSea resolution note: {}. Using raw input.", e);
                            (target_input, None, None)
                        }
                    }
                } else {
                    (target_input, None, None)
                };

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

                // Check on-chain SeaDrop parameters if applicable
                let mut seadrop_contract = None;
                let mut seadrop_fee = None;
                let seadrop_inspector = crate::eligibility::seadrop::SeaDropInspector::new(&rpc_url);
                if let Some(sd) = seadrop_inspector.fetch_public_drop(&target_addr).await {
                    println!("🌊 Detected SeaDrop protocol drop! Price: {} Wei ({:.6} ETH), Max per wallet: {}", sd.mint_price_wei, sd.mint_price_wei as f64 / 1e18, sd.max_per_wallet);
                    detected_price_wei = Some(sd.mint_price_wei.to_string());
                    let clean_fee = sd.fee_recipient.clone().unwrap_or_else(|| "0x0000a26b00c1f0df003000390027140000faa719".to_string());
                    detected_mint_func = Some(format!("mintPublic(address,address,address,uint256) | Fee: {}", clean_fee));
                    seadrop_contract = Some(crate::eligibility::seadrop::SEADROP_V1_ADDRESS.to_string());
                    seadrop_fee = Some(clean_fee);
                }

                // 4. Mint Function Signature (Optional: Enter for default)
                let default_mint_func = detected_mint_func.unwrap_or_else(|| base_config.drop.mint_function.clone());
                print!("4️⃣  Enter Mint Function (Press Enter for '{}'): ", default_mint_func);
                io::stdout().flush()?;
                let mut mint_sig = String::new();
                io::stdin().read_line(&mut mint_sig)?;
                let mint_function = if mint_sig.trim().is_empty() {
                    default_mint_func
                } else {
                    mint_sig.trim().to_string()
                };

                // 5. Mint Value in Wei (Optional: Enter for default)
                let default_val_wei = detected_price_wei.unwrap_or_else(|| base_config.drop.mint_value_wei.clone());
                print!("5️⃣  Enter Mint Value in Wei (Press Enter for '{}'): ", default_val_wei);
                io::stdout().flush()?;
                let mut val_input = String::new();
                io::stdin().read_line(&mut val_input)?;
                let mint_value_wei = if val_input.trim().is_empty() {
                    default_val_wei
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
                session_config.drop.seadrop_contract = seadrop_contract;
                session_config.drop.seadrop_fee_recipient = seadrop_fee;
                session_config.gas.max_fee_gwei = max_gas_fee;

                println!("\n🔒 Session parameters locked into memory buffer. Booting engine...");
                return Ok(MenuAction::Start { key, session_config: Box::new(session_config) });
            }
            "2" => {
                println!("\n--- 🔍 Mint & Wallet Eligibility Checker ---");
                print!("🎯 Enter Contract Address or OpenSea URL (e.g. https://opensea.io/collection/housecraft or 0x...): ");
                io::stdout().flush()?;
                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
                let target_input = input.trim().to_string();
                if target_input.is_empty() {
                    println!("❌ Input cannot be empty.");
                    continue;
                }

                print!("👛 Enter Wallet Address to check eligibility against (0x...): ");
                io::stdout().flush()?;
                let mut wallet_in = String::new();
                io::stdin().read_line(&mut wallet_in)?;
                let wallet_addr = wallet_in.trim().to_string();
                if wallet_addr.len() != 42 || !wallet_addr.starts_with("0x") {
                    println!("❌ Invalid wallet address. Must be a 42-character 0x address.");
                    continue;
                }

                let default_rpc = base_config.chain.rpc_urls.first().cloned().unwrap_or_else(|| "https://rpc.mainnet.chain.robinhood.com".to_string());
                print!("🌐 Enter RPC URL (Press Enter for '{}'): ", default_rpc);
                io::stdout().flush()?;
                let mut rpc_in = String::new();
                io::stdin().read_line(&mut rpc_in)?;
                let rpc_url = if rpc_in.trim().is_empty() { default_rpc } else { rpc_in.trim().to_string() };

                println!("\n⏳ Querying OpenSea API, MintGo intelligence, and live on-chain contract state...");
                let checker = EligibilityChecker::new();
                match checker.analyze_drop(
                    &target_input,
                    &wallet_addr,
                    &rpc_url,
                    base_config.chain.chain_id,
                ).await {
                    Ok(report) => {
                        println!("{}", checker.format_report(&report));

                        print!("👉 Would you like to load this contract into the Sniper now? [y/N]: ");
                        io::stdout().flush()?;
                        let mut launch_in = String::new();
                        io::stdin().read_line(&mut launch_in)?;
                        if launch_in.trim().eq_ignore_ascii_case("y") {
                            println!("\n🔒 Paste your raw private key (Hex) to arm RAM sniper:");
                            let pk_input = if io::stdin().is_terminal() {
                                rpassword::prompt_password("1️⃣  Paste raw private key (Hex): ").unwrap_or_default()
                            } else {
                                let mut l = String::new();
                                let _ = io::stdin().read_line(&mut l);
                                l.trim().to_string()
                            };
                            let pk_guard = zeroize::Zeroizing::new(pk_input);
                            let clean_hex = pk_guard.as_str().trim().trim_start_matches("0x");
                            if let Ok(key) = ProtectedKey::from_hex(clean_hex) {
                                let mut session_config = base_config.clone();
                                session_config.chain.chain_id = report.target.chain_id;
                                session_config.chain.rpc_urls = vec![rpc_url];
                                session_config.drop.target_contract = report.target.contract_address;
                                session_config.drop.mint_value_wei = report.wallet_status.price_wei.to_string();

                                if report.drop_mechanism.contains("SeaDrop") {
                                    session_config.drop.seadrop_contract = Some(crate::eligibility::seadrop::SEADROP_V1_ADDRESS.to_string());
                                    session_config.drop.seadrop_fee_recipient = report.seadrop_config.as_ref().and_then(|s| s.fee_recipient.clone())
                                        .or(report.target.fee_recipient.clone());
                                    session_config.drop.mint_function = "mintPublic(address,address,address,uint256)".to_string();
                                }

                                println!("\n🚀 Loading verified drop into sniper. Booting engine...");
                                return Ok(MenuAction::Start { key, session_config: Box::new(session_config) });
                            } else {
                                println!("❌ Invalid private key. Returning to menu.");
                            }
                        }
                    }
                    Err(e) => {
                        println!("❌ Eligibility analysis error: {e}");
                    }
                }
            }
            "3" => {
                println!("Goodbye!");
                return Ok(MenuAction::Exit);
            }
            _ => println!("❌ Invalid option. Try again."),
        }
    }
}

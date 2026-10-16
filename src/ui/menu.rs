use std::io::{self, IsTerminal, Write};

use crate::config::AppConfig;
use crate::crypto::ProtectedKey;
use crate::eligibility::checker::EligibilityChecker;
use crate::eligibility::opensea::OpenSeaClient;

pub enum MenuAction {
    Start {
        keys: Vec<ProtectedKey>,
        session_config: Box<AppConfig>,
    },
    Exit,
}

/// Reads one line from stdin. Returns `None` on EOF so callers can exit instead of looping.
fn read_line_opt() -> io::Result<Option<String>> {
    let mut line = String::new();
    if io::stdin().read_line(&mut line)? == 0 {
        return Ok(None);
    }
    Ok(Some(line))
}

/// Reads a private key without echo on a TTY. Non-TTY input is refused unless
/// `PULSE_ALLOW_PIPED_KEY=1` is set, to avoid echoing keys into logs or shells.
fn read_private_key(prompt: &str) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
    if io::stdin().is_terminal() {
        return Ok(Some(rpassword::prompt_password(prompt)?));
    }
    if std::env::var("PULSE_ALLOW_PIPED_KEY").as_deref() != Ok("1") {
        return Err("Refusing to read a private key from non-interactive stdin. Set PULSE_ALLOW_PIPED_KEY=1 to override.".into());
    }
    print!("{prompt}");
    io::stdout().flush()?;
    let mut line = String::new();
    if io::stdin().read_line(&mut line)? == 0 {
        return Ok(None);
    }
    Ok(Some(line.trim().to_string()))
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

        let Some(choice) = read_line_opt()? else {
            return Ok(MenuAction::Exit);
        };

        match choice.trim() {
            "1" => {
                println!("\n--- ⚡ Ephemeral RAM Configuration (Zero Disk Saves) ---");
                println!("🔒 Everything entered here lives only in RAM for this active session and is wiped on exit.\n");

                // 1. Private Key(s) (RAM only, hidden input via rpassword when interactive, stdin when piped)
                let pk_input = match read_private_key("1️⃣  Paste raw private key(s) (Hex, comma-separated for multi-wallet): ") {
                    Ok(Some(p)) => p,
                    Ok(None) => return Ok(MenuAction::Exit),
                    Err(e) => {
                        println!("❌ Failed to read private key: {}", e);
                        continue;
                    }
                };
                let pk_guard = zeroize::Zeroizing::new(pk_input);
                let raw_keys: Vec<&str> = pk_guard
                    .as_str()
                    .split(',')
                    .map(|s| s.trim().trim_start_matches("0x"))
                    .filter(|s| !s.is_empty())
                    .collect();

                if raw_keys.is_empty() {
                    println!("❌ No private keys provided.");
                    continue;
                }

                let mut keys = Vec::new();
                let mut parse_failed = false;
                for k in raw_keys {
                    if k.len() != 64 {
                        println!("❌ Invalid private key length ({} chars). Each must be 64 hex characters.", k.len());
                        parse_failed = true;
                        break;
                    }
                    match ProtectedKey::from_hex(k) {
                        Ok(key) => keys.push(key),
                        Err(e) => {
                            println!("❌ Invalid hex key: {}", e);
                            parse_failed = true;
                            break;
                        }
                    }
                }
                if parse_failed || keys.is_empty() {
                    continue;
                }
                println!("🔑 Successfully parsed {} ephemeral wallet key(s) into RAM.", keys.len());

                // 2. Target Contract Address or OpenSea URL
                print!("2️⃣  Enter Target NFT Contract Address or OpenSea URL: ");
                io::stdout().flush()?;
                let Some(target) = read_line_opt()? else {
                    return Ok(MenuAction::Exit);
                };
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
                            println!("❌ Failed to resolve OpenSea slug ({}). Please enter the 42-character 0x contract address directly.", e);
                            continue;
                        }
                    }
                } else {
                    if !target_input.starts_with("0x") || target_input.len() != 42 {
                        println!("❌ Invalid contract address: expected 42-character 0x address or valid OpenSea collection link.");
                        continue;
                    }
                    (target_input, None, None)
                };

                // 3. RPC URLs (Optional: Enter to keep configured pool)
                let default_rpcs = base_config.chain.rpc_urls.join(", ");
                print!("3️⃣  Enter RPC URLs (comma-separated, or Press Enter to keep '{}'): ", default_rpcs);
                io::stdout().flush()?;
                let Some(rpc_input) = read_line_opt()? else {
                    return Ok(MenuAction::Exit);
                };
                let rpc_urls: Vec<String> = if rpc_input.trim().is_empty() {
                    base_config.chain.rpc_urls.clone()
                } else {
                    rpc_input.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
                };
                let primary_rpc = rpc_urls.first().cloned().unwrap_or_else(|| "https://rpc.mainnet.chain.robinhood.com".to_string());

                // Check on-chain SeaDrop parameters if applicable
                let mut seadrop_contract = None;
                let mut seadrop_fee = None;
                let custom_sd = base_config.drop.seadrop_contract.as_deref().unwrap_or(crate::eligibility::seadrop::SEADROP_V1_ADDRESS);
                let seadrop_inspector = crate::eligibility::seadrop::SeaDropInspector::with_address(&primary_rpc, custom_sd);
                if let Some(sd) = seadrop_inspector.fetch_public_drop(&target_addr).await {
                    println!("🌊 Detected SeaDrop protocol drop! Price: {} Wei ({:.6} ETH), Max per wallet: {}", sd.mint_price_wei, sd.mint_price_wei as f64 / 1e18, sd.max_per_wallet);
                    detected_price_wei = Some(sd.mint_price_wei.to_string());
                    let clean_fee = sd.fee_recipient.clone().unwrap_or_else(|| "0x0000a26b00c1f0df003000390027140000faa719".to_string());
                    detected_mint_func = Some("mintPublic(address,address,address,uint256)".to_string());
                    seadrop_contract = Some(seadrop_inspector.seadrop_address.clone());
                    seadrop_fee = Some(clean_fee);
                }

                // 4. Mint Function Signature (Optional: Enter for default)
                let default_mint_func = detected_mint_func.unwrap_or_else(|| base_config.drop.mint_function.clone());
                print!("4️⃣  Enter Mint Function (Press Enter for '{}'): ", default_mint_func);
                io::stdout().flush()?;
                let Some(mint_sig) = read_line_opt()? else {
                    return Ok(MenuAction::Exit);
                };
                let mint_function = if mint_sig.trim().is_empty() {
                    default_mint_func
                } else {
                    mint_sig.trim().to_string()
                };

                // 5. Mint Value in Wei (Optional: Enter for default). Invalid input re-prompts.
                let default_val_wei = detected_price_wei.unwrap_or_else(|| base_config.drop.mint_value_wei.clone());
                let mint_value_wei = loop {
                    print!("5️⃣  Enter Mint Value in Wei (Press Enter for '{}'): ", default_val_wei);
                    io::stdout().flush()?;
                    let Some(val_input) = read_line_opt()? else {
                        return Ok(MenuAction::Exit);
                    };
                    let trimmed = val_input.trim();
                    if trimmed.is_empty() {
                        break default_val_wei.clone();
                    }
                    if trimmed.parse::<u128>().is_ok() {
                        break trimmed.to_string();
                    }
                    println!("❌ Invalid mint value '{trimmed}'. Enter an integer number of Wei (e.g. 0).");
                };

                // 6. Mint Quantity (Optional: Enter for default). Invalid input re-prompts.
                let default_qty = base_config.drop.mint_quantity;
                let mint_quantity = loop {
                    print!("6️⃣  Enter Mint Quantity (Press Enter for '{}'): ", default_qty);
                    io::stdout().flush()?;
                    let Some(qty_input) = read_line_opt()? else {
                        return Ok(MenuAction::Exit);
                    };
                    let trimmed = qty_input.trim();
                    if trimmed.is_empty() {
                        break default_qty;
                    }
                    match trimmed.parse::<u64>() {
                        Ok(q) if q > 0 => break q,
                        _ => println!("❌ Invalid quantity '{trimmed}'. Enter a positive integer."),
                    }
                };

                // 7. Max Gas Fee in Gwei (Optional: Enter for default). Invalid input re-prompts.
                let max_gas_fee = loop {
                    print!("7️⃣  Enter Max Gas Fee in Gwei (Press Enter for '{:.1}'): ", base_config.gas.max_fee_gwei);
                    io::stdout().flush()?;
                    let Some(gas_input) = read_line_opt()? else {
                        return Ok(MenuAction::Exit);
                    };
                    let trimmed = gas_input.trim();
                    if trimmed.is_empty() {
                        break base_config.gas.max_fee_gwei;
                    }
                    match trimmed.parse::<f64>() {
                        Ok(v) if v.is_finite() && v > 0.0 => break v,
                        _ => println!("❌ Invalid gas fee '{trimmed}'. Enter a positive number of Gwei."),
                    }
                };

                // 8. Snipe Trigger Mode Selection
                println!("\n🎯 Select Sniping Trigger Mode:");
                println!("   a) Instant / State Poller (Watches when contract sale flips active)");
                println!("   b) Countdown Timer (Target Unix timestamp)");
                println!("   c) Mempool Backrun (Watches owner transaction)");
                print!("👉 Choose trigger mode [a/b/c] (default 'a'): ");
                io::stdout().flush()?;
                let Some(trigger_choice) = read_line_opt()? else {
                    return Ok(MenuAction::Exit);
                };

                let mut target_timestamp = None;
                let mut monitor_owner_address = None;
                let trigger_mode = match trigger_choice.trim().to_lowercase().as_str() {
                    "b" => {
                        print!("   Enter Target Unix Timestamp: ");
                        io::stdout().flush()?;
                        let Some(ts_str) = read_line_opt()? else {
                            return Ok(MenuAction::Exit);
                        };
                        target_timestamp = ts_str.trim().parse::<u64>().ok();
                        crate::config::TriggerMode::Countdown
                    }
                    "c" => {
                        print!("   Enter Owner Address to Monitor: ");
                        io::stdout().flush()?;
                        let Some(owner_str) = read_line_opt()? else {
                            return Ok(MenuAction::Exit);
                        };
                        if !owner_str.trim().is_empty() {
                            monitor_owner_address = Some(owner_str.trim().to_string());
                        }
                        crate::config::TriggerMode::Mempool
                    }
                    _ => crate::config::TriggerMode::StatePoller,
                };

                // Construct session configuration strictly in memory (NEVER saved to file)
                let mut session_config = base_config.clone();
                session_config.chain.rpc_urls = rpc_urls;
                session_config.drop.target_contract = target_addr;
                session_config.drop.mint_function = mint_function;
                session_config.drop.mint_value_wei = mint_value_wei;
                session_config.drop.mint_quantity = mint_quantity;
                session_config.drop.trigger_mode = trigger_mode;
                session_config.drop.target_timestamp = target_timestamp;
                session_config.drop.monitor_owner_address = monitor_owner_address;
                session_config.drop.seadrop_contract = seadrop_contract;
                session_config.drop.seadrop_fee_recipient = seadrop_fee;
                session_config.gas.max_fee_gwei = max_gas_fee;

                println!("\n🔒 Session parameters locked into memory buffer. Booting engine...");
                return Ok(MenuAction::Start { keys, session_config: Box::new(session_config) });
            }
            "2" => {
                println!("\n--- 🔍 Mint & Wallet Eligibility Checker ---");
                print!("🎯 Enter Contract Address or OpenSea URL (e.g. https://opensea.io/collection/housecraft or 0x...): ");
                io::stdout().flush()?;
                let Some(input) = read_line_opt()? else {
                    return Ok(MenuAction::Exit);
                };
                let target_input = input.trim().to_string();
                if target_input.is_empty() {
                    println!("❌ Input cannot be empty.");
                    continue;
                }

                print!("👛 Enter Wallet Address to check eligibility against (0x...): ");
                io::stdout().flush()?;
                let Some(wallet_in) = read_line_opt()? else {
                    return Ok(MenuAction::Exit);
                };
                let wallet_addr = wallet_in.trim().to_string();
                if wallet_addr.len() != 42 || !wallet_addr.starts_with("0x") {
                    println!("❌ Invalid wallet address. Must be a 42-character 0x address.");
                    continue;
                }

                let default_rpc = base_config.chain.rpc_urls.first().cloned().unwrap_or_else(|| "https://rpc.mainnet.chain.robinhood.com".to_string());
                print!("🌐 Enter RPC URL (Press Enter for '{}'): ", default_rpc);
                io::stdout().flush()?;
                let Some(rpc_in) = read_line_opt()? else {
                    return Ok(MenuAction::Exit);
                };
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

                        if report.is_sold_out {
                            println!("⛔ Drop is sold out. Sniper not offered.");
                            continue;
                        }
                        if !report.wallet_status.is_eligible {
                            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
                            let is_upcoming = report.seadrop_config.as_ref().is_some_and(|sd| now < sd.start_time)
                                || report.stages.iter().any(|st| st.start_time_unix.is_some_and(|t| now < t));

                            if is_upcoming {
                                println!("⏳ Drop is upcoming (not yet active). You can arm the sniper with a countdown timer.");
                            } else {
                                println!("⛔ Wallet is not eligible ({}). Sniper not offered.", report.wallet_status.status_reason);
                                continue;
                            }
                        }

                        print!("👉 Would you like to load this contract into the Sniper now? [y/N]: ");
                        io::stdout().flush()?;
                        let Some(launch_in) = read_line_opt()? else {
                            return Ok(MenuAction::Exit);
                        };
                        if launch_in.trim().eq_ignore_ascii_case("y") {
                            println!("\n🔒 Paste your raw private key (Hex) to arm RAM sniper:");
                            let pk_input = match read_private_key("1️⃣  Paste raw private key (Hex): ") {
                                Ok(Some(p)) => p,
                                Ok(None) => return Ok(MenuAction::Exit),
                                Err(e) => {
                                    println!("❌ Failed to read private key: {}. Returning to menu.", e);
                                    continue;
                                }
                            };
                            let pk_guard = zeroize::Zeroizing::new(pk_input);
                            let clean_hex = pk_guard.as_str().trim().trim_start_matches("0x");
                            if let Ok(key) = ProtectedKey::from_hex(clean_hex) {
                                let mut session_config = base_config.clone();
                                session_config.chain.chain_id = report.target.chain_id;
                                // Keep the fallback pool; the user's RPC goes first.
                                let mut pool = vec![rpc_url];
                                for url in &base_config.chain.rpc_urls {
                                    if !pool.contains(url) {
                                        pool.push(url.clone());
                                    }
                                }
                                session_config.chain.rpc_urls = pool;
                                session_config.drop.target_contract = report.target.contract_address.clone();
                                session_config.drop.mint_value_wei = report.wallet_status.price_wei.to_string();

                                // Pre-fill countdown timer if drop is upcoming
                                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
                                if let Some(ref sd) = report.seadrop_config {
                                    if sd.start_time > now {
                                        session_config.drop.target_timestamp = Some(sd.start_time);
                                        session_config.drop.trigger_mode = crate::config::TriggerMode::Countdown;
                                        println!("⏱️ Pre-configured countdown timer to start time: {}", sd.start_time);
                                    } else {
                                        session_config.drop.trigger_mode = crate::config::TriggerMode::StatePoller;
                                    }
                                }

                                if report.drop_mechanism.contains("SeaDrop") {
                                    session_config.drop.seadrop_contract = Some(crate::eligibility::seadrop::SEADROP_V1_ADDRESS.to_string());
                                    session_config.drop.seadrop_fee_recipient = report.seadrop_config.as_ref().and_then(|s| s.fee_recipient.clone())
                                        .or(report.target.fee_recipient.clone());
                                    session_config.drop.mint_function = "mintPublic(address,address,address,uint256)".to_string();
                                }

                                println!("\n🚀 Loading verified drop into sniper. Booting engine...");
                                return Ok(MenuAction::Start { keys: vec![key], session_config: Box::new(session_config) });
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

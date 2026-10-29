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

fn parse_hex_hashes(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(|s| s.trim().trim_start_matches("0x").to_string())
        .filter(|s| s.len() == 64 && hex::decode(s).is_ok())
        .map(|s| format!("0x{s}"))
        .collect()
}

fn parse_keys_from_input(input: String) -> Result<Vec<ProtectedKey>, String> {
    let pk_guard = zeroize::Zeroizing::new(input);
    let raw_keys: Vec<&str> = pk_guard
        .as_str()
        .split(',')
        .map(|s| s.trim().trim_start_matches("0x"))
        .filter(|s| !s.is_empty())
        .collect();
    if raw_keys.is_empty() {
        return Err("No private keys provided.".to_string());
    }
    let mut keys = Vec::with_capacity(raw_keys.len());
    for k in raw_keys {
        if k.len() != 64 {
            return Err(format!("Invalid key length ({} chars). Each must be 64 hex characters.", k.len()));
        }
        keys.push(ProtectedKey::from_hex(k).map_err(|e| format!("Invalid hex key: {e}"))?);
    }
    Ok(keys)
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
                let keys = match parse_keys_from_input(pk_input) {
                    Ok(k) => k,
                    Err(e) => {
                        println!("❌ {e}");
                        continue;
                    }
                };
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
                let mut auto_merkle_proof = Vec::new();
                let mut resolved_chain_id = None;
                let (target_addr, mut detected_mint_func, mut detected_price_wei) = if let Some(slug) = OpenSeaClient::extract_slug(&target_input) {
                    println!("🔎 Resolving OpenSea collection slug: '{slug}'...");
                    let mut opensea = OpenSeaClient::new();
                    match opensea.fetch_collection_by_slug(&slug).await {
                        Ok(resolved) => {
                            println!("✅ Resolved OpenSea drop: '{}' -> {} (Chain: {})", resolved.collection_name, resolved.contract_address, resolved.chain);
                            resolved_chain_id = Some(resolved.chain_id);
                            println!("🔑 Attempting OpenSea SIWE auto-auth...");
                            match opensea.authenticate_with_key(&keys[0], resolved.chain_id).await {
                                Ok(_) => {
                                    println!("✅ OpenSea SIWE session authenticated.");
                                    if let Ok(addr) = crate::crypto::get_address_from_protected(&keys[0]) {
                                        if let Ok(elig) = opensea.fetch_drop_eligibility(&slug, &addr).await {
                                            if !elig.merkle_proof.is_empty() {
                                                println!("📜 Auto-loaded {} allowlist Merkle proof hash(es) from OpenSea!", elig.merkle_proof.len());
                                                auto_merkle_proof = elig.merkle_proof;
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    println!("ℹ️ OpenSea automated SIWE auth unavailable ({e}). Fallback to manual Merkle proofs if required.");
                                }
                            }
                            (resolved.contract_address, None, None)
                        }
                        Err(e) => {
                            println!("❌ Failed to resolve OpenSea slug ({e}). Please enter 42-char 0x address directly.");
                            continue;
                        }
                    }
                } else {
                    if !target_input.starts_with("0x") || target_input.len() != 42 {
                        println!("❌ Invalid contract address: expected 42-char 0x address or OpenSea collection link.");
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
                // This is the PER-TOKEN unit price; the engine multiplies it by quantity to
                // build `msg.value` (SeaDrop requires `msg.value == quantity * mintPrice`).
                let default_val_wei = detected_price_wei.unwrap_or_else(|| base_config.drop.mint_value_wei.clone());
                let mint_value_wei = loop {
                    print!("5️⃣  Enter Unit Mint Price in Wei (per token, Press Enter for '{default_val_wei}'): ");
                    io::stdout().flush()?;
                    let Some(val_input) = read_line_opt()? else { return Ok(MenuAction::Exit); };
                    let trimmed = val_input.trim();
                    if trimmed.is_empty() { break default_val_wei.clone(); }
                    if trimmed.parse::<u128>().is_ok() { break trimmed.to_string(); }
                    println!("❌ Invalid mint value '{trimmed}'. Enter an integer number of Wei (e.g. 0).");
                };

                // 6. Mint Quantity (Optional: Enter for default). Invalid input re-prompts.
                let default_qty = base_config.drop.mint_quantity;
                let mint_quantity = loop {
                    print!("6️⃣  Enter Mint Quantity (Press Enter for '{default_qty}'): ");
                    io::stdout().flush()?;
                    let Some(qty_input) = read_line_opt()? else { return Ok(MenuAction::Exit); };
                    let trimmed = qty_input.trim();
                    if trimmed.is_empty() { break default_qty; }
                    match trimmed.parse::<u64>() {
                        Ok(q) if q > 0 => break q,
                        _ => println!("❌ Invalid quantity '{trimmed}'. Enter a positive integer."),
                    }
                };

                // Surface the total native value up front: this is what actually leaves the
                // wallet, and a mismatch with the contract's expectation reverts the mint.
                if let Ok(unit) = mint_value_wei.trim().parse::<u128>() {
                    match unit.checked_mul(mint_quantity as u128) {
                        Some(total) if total > 0 => println!(
                            "💸 Total native value per wallet: {} Wei ({:.6} ETH) = {} x {}",
                            total,
                            total as f64 / 1e18,
                            mint_quantity,
                            unit
                        ),
                        Some(_) => {}
                        None => println!("❌ Unit price {} Wei x quantity {} overflows; reduce either value.", unit, mint_quantity),
                    }
                }

                // 7. Max Gas Fee in Gwei (Optional: Enter for default). Invalid input re-prompts.
                let max_gas_fee = loop {
                    print!("7️⃣  Enter Max Gas Fee in Gwei (Press Enter for '{:.1}'): ", base_config.gas.max_fee_gwei);
                    io::stdout().flush()?;
                    let Some(gas_input) = read_line_opt()? else { return Ok(MenuAction::Exit); };
                    let trimmed = gas_input.trim();
                    if trimmed.is_empty() { break base_config.gas.max_fee_gwei; }
                    match trimmed.parse::<f64>() {
                        Ok(v) if v.is_finite() && v > 0.0 => break v,
                        _ => println!("❌ Invalid gas fee '{trimmed}'. Enter a positive number of Gwei."),
                    }
                };

                // 8. Snipe Trigger Mode Selection
                println!("\n🎯 Select Sniping Trigger Mode:\n   a) Instant / State Poller (Watches when contract sale flips active)\n   b) Countdown Timer (Target Unix timestamp)\n   c) Mempool Backrun (Watches owner transaction)");
                print!("👉 Choose trigger mode [a/b/c] (default 'a'): ");
                io::stdout().flush()?;
                let Some(trigger_choice) = read_line_opt()? else { return Ok(MenuAction::Exit); };

                let mut target_timestamp = None;
                let mut monitor_owner_address = None;
                let trigger_mode = match trigger_choice.trim().to_lowercase().as_str() {
                    "b" => {
                        print!("   Enter Target Unix Timestamp: ");
                        io::stdout().flush()?;
                        let Some(ts_str) = read_line_opt()? else { return Ok(MenuAction::Exit); };
                        target_timestamp = ts_str.trim().parse::<u64>().ok();
                        crate::config::TriggerMode::Countdown
                    }
                    "c" => {
                        print!("   Enter Owner Address to Monitor: ");
                        io::stdout().flush()?;
                        let Some(owner_str) = read_line_opt()? else { return Ok(MenuAction::Exit); };
                        if !owner_str.trim().is_empty() { monitor_owner_address = Some(owner_str.trim().to_string()); }
                        crate::config::TriggerMode::Mempool
                    }
                    _ => crate::config::TriggerMode::StatePoller,
                };

                // 9. Merkle Proof Hashes (for allowlist / whitelist drops)
                let is_allowlist = mint_function.contains("bytes32[]")
                    || mint_function.to_lowercase().contains("whitelist")
                    || mint_function.to_lowercase().contains("allowlist")
                    || seadrop_contract.is_some();
                let mut merkle_proof = if !auto_merkle_proof.is_empty() {
                    auto_merkle_proof
                } else {
                    base_config.drop.merkle_proof.clone()
                };
                if is_allowlist && merkle_proof.is_empty() {
                    print!("📜 Enter Merkle Proof hashes (comma-separated hex or press Enter to skip): ");
                    io::stdout().flush()?;
                    if let Some(proof_str) = read_line_opt()? {
                        let hashes = parse_hex_hashes(&proof_str);
                        if !hashes.is_empty() {
                            println!("✅ Loaded {} valid Merkle proof hash(es).", hashes.len());
                            merkle_proof = hashes;
                        }
                    }
                }

                // Construct session configuration strictly in memory (NEVER saved to file)
                let mut session_config = base_config.clone();
                if let Some(cid) = resolved_chain_id {
                    session_config.chain.chain_id = cid;
                }
                session_config.chain.rpc_urls = rpc_urls;
                session_config.drop.target_contract = target_addr;
                session_config.drop.mint_function = mint_function;
                session_config.drop.mint_value_wei = mint_value_wei;
                session_config.drop.mint_quantity = mint_quantity;
                session_config.drop.trigger_mode = trigger_mode;
                session_config.drop.merkle_proof = merkle_proof;
                session_config.drop.target_timestamp = target_timestamp;
                session_config.drop.monitor_owner_address = monitor_owner_address;
                if seadrop_contract.is_some() && !session_config.drop.merkle_proof.is_empty() {
                    session_config.drop.allowlist_stage_index = session_config.drop.allowlist_stage_index.or(Some(1));
                }
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
                            println!("\n🔒 Paste your raw private key(s) (Hex, comma-separated for multi-wallet):");
                            let pk_input = match read_private_key("1️⃣  Paste raw private key(s) (Hex): ") {
                                Ok(Some(p)) => p,
                                Ok(None) => return Ok(MenuAction::Exit),
                                Err(e) => {
                                    println!("❌ Failed to read private key: {e}. Returning to menu.");
                                    continue;
                                }
                            };
                            let keys = match parse_keys_from_input(pk_input) {
                                Ok(k) => k,
                                Err(e) => {
                                    println!("❌ {e}. Returning to menu.");
                                    continue;
                                }
                            };
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
                                session_config.drop.allowlist_end_time = Some(sd.end_time);
                                session_config.drop.allowlist_fee_bps = Some(sd.fee_bps);
                                session_config.drop.allowlist_wallet_limit = Some(sd.max_per_wallet);
                                if sd.start_time > now {
                                    session_config.drop.target_timestamp = Some(sd.start_time);
                                    session_config.drop.trigger_mode = crate::config::TriggerMode::Countdown;
                                    println!("⏱️ Pre-configured countdown timer to start time: {}", sd.start_time);
                                } else {
                                    session_config.drop.trigger_mode = crate::config::TriggerMode::StatePoller;
                                }
                            }

                            let has_allowlist = report.stages.iter().any(|s| {
                                s.stage_type.to_lowercase().contains("allowlist")
                                    || s.stage_type.to_lowercase().contains("whitelist")
                                    || s.label.to_lowercase().contains("allowlist")
                                    || s.label.to_lowercase().contains("whitelist")
                            });
                            if let Some(ref oe) = report.opensea_eligibility {
                                if !oe.merkle_proof.is_empty() {
                                    session_config.drop.merkle_proof = oe.merkle_proof.clone();
                                    session_config.drop.allowlist_wallet_limit = oe.max_mintable;
                                    println!("📜 Pre-loaded {} Merkle proof hash(es) from OpenSea report.", oe.merkle_proof.len());
                                }
                            }
                            if session_config.drop.merkle_proof.is_empty() && (has_allowlist || report.drop_mechanism.contains("SeaDrop")) {
                                print!("📜 Enter Merkle Proof hashes (comma-separated hex or press Enter to skip): ");
                                io::stdout().flush()?;
                                if let Some(proof_str) = read_line_opt()? {
                                    let hashes = parse_hex_hashes(&proof_str);
                                    if !hashes.is_empty() {
                                        println!("✅ Loaded {} valid Merkle proof hash(es).", hashes.len());
                                        session_config.drop.merkle_proof = hashes;
                                    }
                                }
                            }

                            if report.drop_mechanism.contains("SeaDrop") {
                                session_config.drop.seadrop_contract = Some(crate::eligibility::seadrop::SEADROP_V1_ADDRESS.to_string());
                                session_config.drop.seadrop_fee_recipient = report.seadrop_config.as_ref().and_then(|s| s.fee_recipient.clone())
                                    .or(report.target.fee_recipient.clone());
                                if !session_config.drop.merkle_proof.is_empty() {
                                    session_config.drop.allowlist_stage_index = session_config.drop.allowlist_stage_index.or(Some(1));
                                    session_config.drop.mint_function = "mintAllowList(address,address,address,uint256,(uint256,uint256,uint256,uint256,uint256,uint256,uint256,bool),bytes32[])".to_string();
                                } else {
                                    session_config.drop.mint_function = "mintPublic(address,address,address,uint256)".to_string();
                                }
                            } else if let Some(ref detected_fn) = report.wallet_status.detected_mint_function {
                                session_config.drop.mint_function = detected_fn.clone();
                            }

                            println!("\n🚀 Loading verified drop into sniper. Booting engine...");
                            return Ok(MenuAction::Start { keys, session_config: Box::new(session_config) });
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

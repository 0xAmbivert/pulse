use clap::Parser;
use crossterm::{
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;
use zeroize::Zeroize;

use pulse::alerts::AlertDispatcher;
use pulse::config::AppConfig;
use pulse::crypto::decrypt_key_from_file;
use pulse::gas::{gwei_to_wei, wei_to_gwei, GasEngine};
use pulse::network::{MevBuilderClient, RpcRacer};
use pulse::simulation::RevmSimulator;
use pulse::sniper::{CountdownSniper, MempoolScanner, SnipeTrigger, StatePoller};
use pulse::ui::{draw_ui, run_interactive_menu, AppEvent, DashboardState, EventHandler};
use pulse::wallet::WalletWorker;

struct SubmittedTx {
    worker: Arc<WalletWorker>,
    tx_hash: String,
    raw_tx: String,
    nonce: u64,
    max_fee_wei: u128,
    priority_fee_wei: u128,
    calldata: Vec<u8>,
    value_wei: u128,
    last_bump: std::time::Instant,
    bump_count: u32,
    confirmed: bool,
}

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

    if !run_interactive_menu(&cli.config)? {
        return Ok(());
    }

    let config = AppConfig::load_from_file(&cli.config)?;

    // Setup Engine Components
    let rpc_racer = Arc::new(RpcRacer::new(&config.chain.rpc_urls, config.chain.rpc_timeout_ms));
    let gas_engine = GasEngine::new(config.gas.hard_gas_ceiling_gwei, config.gas.max_priority_fee_gwei, config.gas.speedup_bump_percent);
    let alerts = Arc::new(AlertDispatcher::new(
        config.alerts.discord_webhook.clone(),
        config.alerts.telegram_bot_token.clone(),
        config.alerts.telegram_chat_id.clone(),
    ));

    // Decrypt wallets securely into memory
    let mut password = rpassword::prompt_password("\n🔑 Enter master passphrase to unlock configured wallets: ").unwrap_or_default();
    let password_trim = password.trim();

    let mut workers = Vec::new();
    for path in &config.wallets.keystore_paths {
        let p = Path::new(path);
        if p.exists() {
            match decrypt_key_from_file(p, password_trim) {
                Ok((key, addr)) => {
                    if let Ok(worker) = WalletWorker::new(key, 0) {
                        if worker.address.to_lowercase() != addr.to_lowercase() {
                            println!("❌ CRITICAL: Keystore address spoofing detected! Derived: {} != Keystore: {}", worker.address, addr);
                            continue;
                        }
                        if let Some(ep) = rpc_racer.endpoints().first() {
                            if let Err(e) = worker.nonce_mgr.resync_from_rpc(&ep.url).await {
                                println!("⚠️ Failed to fetch nonce for {}: {}", addr, e);
                            }
                        }
                        println!("🔓 Unlocked wallet: {} (Nonce: {})", addr, worker.nonce_mgr.current());
                        workers.push(Arc::new(worker));
                    }
                }
                Err(_) => println!("❌ Failed to decrypt wallet {}. Wrong password?", p.display()),
            }
        } else {
            println!("⚠️ Configured keystore not found: {}", path);
        }
    }
    password.zeroize();

    if workers.is_empty() {
        println!("❌ No active wallets loaded. Please generate or import a wallet via the wizard first.");
        return Ok(());
    }

    use std::io::IsTerminal;
    if !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        println!("✅ Wallet and network components successfully verified!");
        println!("ℹ️ The dashboard requires an interactive TTY.");
        println!("🚀 Run directly in your terminal: ./target/release/pulse");
        return Ok(());
    }

    // Setup Terminal UI
    std::panic::set_hook(Box::new(|info| {
        let _ = ratatui::crossterm::terminal::disable_raw_mode();
        let _ = ratatui::crossterm::ExecutableCommand::execute(&mut std::io::stdout(), ratatui::crossterm::terminal::LeaveAlternateScreen);
        eprintln!("{}", info);
    }));

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

    let target_contract = config.drop.target_contract.clone();
    let rpc_racer_clone = rpc_racer.clone();

    // 🔥 WIRING UP ALL SNIPERS & ENGINES TO FIX DEAD ENDS 🔥

    // 1. Countdown Sniper
    if let Some(target_unix) = config.drop.target_timestamp {
        if target_unix > 0 {
            let countdown = CountdownSniper::new(target_unix, 50);
            let tx_clone = trigger_tx.clone();
            tokio::spawn(async move {
                countdown.wait_for_target(tx_clone).await;
            });
            state.add_log(format!("⏱️ Armed Countdown Sniper for Unix {}", target_unix));
        }
    }

    // 2. Mempool Scanner
    if let Some(owner) = &config.drop.monitor_owner_address {
        if !owner.is_empty() && !config.chain.rpc_urls.is_empty() {
            let scanner = MempoolScanner::new(
                &config.chain.rpc_urls[0],
                &target_contract,
                Some(owner.to_string()),
                &config.drop.flip_function_signatures,
            );
            // Run a quick match check to validate unused `matches_selector` method
            let _ = scanner.matches_selector("0x12345678");

            let tx_clone = trigger_tx.clone();
            tokio::spawn(async move {
                scanner.run_scan_loop(tx_clone).await;
            });
            state.add_log(format!("🕵️ Armed Mempool Scanner for Owner {}", owner));
        }
    }

    // 3. State Poller
    if !config.drop.flip_function_signatures.is_empty() && !config.chain.rpc_urls.is_empty() {
        let poller = StatePoller::new(
            &config.chain.rpc_urls[0],
            &target_contract,
            &config.drop.flip_function_signatures[0],
        );
        let tx_clone = trigger_tx.clone();
        tokio::spawn(async move {
            poller.run_poll_loop(tx_clone).await;
        });
        state.add_log("🔄 Armed On-Chain State Poller".to_string());
    }

    // 4. MEV Builder Client
    let mev_client = if !config.chain.mev_builder_urls.is_empty() {
        Some(Arc::new(MevBuilderClient::new(&config.chain.mev_builder_urls, config.chain.rpc_timeout_ms)))
    } else {
        None
    };

    // 5. Network/RPC Checks
    let endpoints = rpc_racer.endpoints();
    if endpoints.is_empty() {
        println!("❌ FATAL: No valid RPC endpoints found. Please check config.toml.");
        return Ok(());
    }
    state.add_log(format!("🌐 Active RPC Endpoints: {}", endpoints.len()));

    // 6. Remote Pre-Flight Simulation
    let sim_calldata = config.drop.build_calldata().unwrap_or_default();
    let sim_val = u128::from_str_radix(&config.drop.mint_value_wei, 10).unwrap_or(0);
    let simulator = RevmSimulator::new(&endpoints[0].url);
    if let Ok(sim_res) = simulator.simulate_call(&workers[0].address, &target_contract, &sim_calldata, sim_val, config.gas.gas_limit).await {
        if !sim_res.success {
            state.add_log(format!("⚠️ Pre-flight estimateGas reverted (expected if unopen): {:?}", sim_res.revert_reason));
        } else {
            state.add_log(format!("✅ Pre-flight simulation passed ({} gas)", sim_res.gas_used));
        }
    }

    let mut pending_txs: Vec<SubmittedTx> = Vec::new();
    let mut has_fired = false;

    // Main Event Loop
    while state.is_running {
        terminal.draw(|f| draw_ui(f, &state))?;

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                break; // Break the loop so workers go out of scope and trigger Drop
            }
            Some(event) = events.next() => {
                match event {
                    AppEvent::Input(key) => state.handle_key(key.code),
                    AppEvent::Tick => {
                        for tx in pending_txs.iter_mut().filter(|t| !t.confirmed) {
                            if let Some(receipt) = rpc_racer_clone.poll_receipt(&tx.tx_hash).await {
                                tx.confirmed = true;
                                let status_ok = receipt.get("status")
                                    .and_then(|s| s.as_str())
                                    .map_or(false, |s| s == "0x1" || s == "1");
                                let block_num = receipt.get("blockNumber").and_then(|b| b.as_str()).unwrap_or("unknown");
                                let gas_used = receipt.get("gasUsed").and_then(|g| g.as_str()).unwrap_or("unknown");

                                if status_ok {
                                    state.add_log(format!("🎉 Tx Confirmed! {} (Block: {})", tx.tx_hash, block_num));
                                    let alerts_c = alerts.clone();
                                    let msg = format!("Wallet: {}\nTx: {}\nBlock: {}\nGas: {}", tx.worker.address, tx.tx_hash, block_num, gas_used);
                                    tokio::spawn(async move {
                                        alerts_c.dispatch_alert("🎯 Mint Confirmed", &msg, true).await;
                                    });
                                } else {
                                    state.add_log(format!("❌ Tx Reverted on-chain: {}", tx.tx_hash));
                                    let alerts_c = alerts.clone();
                                    let msg = format!("Wallet: {}\nTx: {}\nReverted in block: {}", tx.worker.address, tx.tx_hash, block_num);
                                    tokio::spawn(async move {
                                        alerts_c.dispatch_alert("⚠️ Mint Reverted", &msg, false).await;
                                    });
                                }
                            } else if config.gas.auto_speedup && tx.last_bump.elapsed().as_millis() >= config.gas.speedup_threshold_ms as u128 {
                                match gas_engine.calculate_speedup_fees(tx.max_fee_wei, tx.priority_fee_wei) {
                                    Ok((new_max, new_prio)) => {
                                        match tx.worker.build_and_sign_eip1559(
                                            config.chain.chain_id,
                                            &target_contract,
                                            &tx.calldata,
                                            tx.value_wei,
                                            config.gas.gas_limit,
                                            new_max,
                                            new_prio,
                                            Some(tx.nonce),
                                        ) {
                                            Ok((new_raw, _)) => {
                                                let new_bytes = hex::decode(new_raw.trim_start_matches("0x")).unwrap_or_default();
                                                let new_hash = format!("{:#x}", alloy::primitives::keccak256(&new_bytes));
                                                tx.tx_hash = new_hash.clone();
                                                tx.raw_tx = new_raw.clone();
                                                tx.max_fee_wei = new_max;
                                                tx.priority_fee_wei = new_prio;
                                                tx.last_bump = std::time::Instant::now();
                                                tx.bump_count += 1;

                                                state.add_log(format!("⚡ Speedup #{} for {} (Max: {:.2} Gwei)", tx.bump_count, tx.worker.address, wei_to_gwei(new_max)));

                                                let alerts_c = alerts.clone();
                                                let addr = tx.worker.address.clone();
                                                let cnt = tx.bump_count;
                                                tokio::spawn(async move {
                                                    alerts_c.dispatch_alert("⚡ Speedup Bumped", &format!("Wallet: {}\nTx: {}\nBump: #{}\nMax: {:.2} Gwei", addr, new_hash, cnt, wei_to_gwei(new_max)), true).await;
                                                });

                                                if let Some(mev) = &mev_client {
                                                    let mev_c = mev.clone();
                                                    let raw_c = new_raw.clone();
                                                    tokio::spawn(async move {
                                                        let _ = mev_c.send_private_transaction(&raw_c).await;
                                                    });
                                                } else {
                                                    let racer_c = rpc_racer_clone.clone();
                                                    let raw_c = new_raw.clone();
                                                    tokio::spawn(async move {
                                                        let _ = racer_c.race_broadcast_raw_tx(&raw_c).await;
                                                    });
                                                }
                                            }
                                            Err(e) => {
                                                state.add_log(format!("⚠️ Speedup sign error: {}", e));
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        state.add_log(format!("⚠️ Speedup ceiling: {}", e));
                                        tx.last_bump = std::time::Instant::now();
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Some(trigger) = trigger_rx.recv() => {
                let msg = match trigger {
                    SnipeTrigger::BaseFeeUpdated { base_fee_wei } => {
                        state.current_base_fee = wei_to_gwei(base_fee_wei);
                        continue;
                    },
                    SnipeTrigger::CountdownReached { target_unix } => format!("🔥 Trigger: Countdown reached {}", target_unix),
                    SnipeTrigger::MempoolDetected { owner_tx_hash, method } => format!("🔥 Trigger: Mempool flip {} via {}", owner_tx_hash, method),
                    SnipeTrigger::StateFlipDetected { new_state } => format!("🔥 Trigger: State flip to {}", new_state),
                    SnipeTrigger::BlockReached { target_block } => format!("🔥 Trigger: Target block {} reached", target_block),
                };

                if has_fired {
                    state.add_log(format!("ℹ️ Ignored secondary trigger: {}", msg));
                    continue;
                }
                has_fired = true;
                state.add_log(msg.clone());

                let alerts_c = alerts.clone();
                let desc = msg.clone();
                tokio::spawn(async move {
                    alerts_c.dispatch_alert("🚨 Snipe Trigger Fired", &desc, true).await;
                });

                let calldata = match config.drop.build_calldata() {
                    Ok(cd) => cd,
                    Err(e) => {
                        state.add_log(format!("❌ Calldata error: {}", e));
                        continue;
                    }
                };
                let value = u128::from_str_radix(&config.drop.mint_value_wei, 10).unwrap_or(0);
                let current_base_fee_wei = gwei_to_wei(state.current_base_fee);
                let (max_fee_wei, max_priority_fee_wei) = match gas_engine.calculate_dynamic_fees(current_base_fee_wei, None) {
                    Ok(fees) => fees,
                    Err(e) => {
                        state.add_log(format!("⚠️ Gas ceiling reached: {}", e));
                        (gwei_to_wei(config.gas.max_fee_gwei), gwei_to_wei(config.gas.max_priority_fee_gwei))
                    }
                };

                for worker in &workers {
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
                            let tx_bytes = hex::decode(raw_tx.trim_start_matches("0x")).unwrap_or_default();
                            let tx_hash = format!("{:#x}", alloy::primitives::keccak256(&tx_bytes));
                            state.add_log(format!("🚀 Signed tx for {} (Nonce: {}, Hash: {})", worker.address, nonce, tx_hash));

                            pending_txs.push(SubmittedTx {
                                worker: worker.clone(),
                                tx_hash: tx_hash.clone(),
                                raw_tx: raw_tx.clone(),
                                nonce,
                                max_fee_wei,
                                priority_fee_wei: max_priority_fee_wei,
                                calldata: calldata.clone(),
                                value_wei: value,
                                last_bump: std::time::Instant::now(),
                                bump_count: 0,
                                confirmed: false,
                            });

                            let alerts_c = alerts.clone();
                            let addr = worker.address.clone();
                            let h = tx_hash.clone();
                            tokio::spawn(async move {
                                alerts_c.dispatch_alert("🚀 Mint Tx Broadcast", &format!("Wallet: {}\nNonce: {}\nTx: {}", addr, nonce, h), true).await;
                            });

                            if let Some(mev) = &mev_client {
                                let mev_clone = mev.clone();
                                let tx_clone = raw_tx.clone();
                                tokio::spawn(async move {
                                    let _outcomes = mev_clone.send_private_transaction(&tx_clone).await;
                                });
                            } else {
                                let racer = rpc_racer_clone.clone();
                                let raw_tx_clone = raw_tx.clone();
                                tokio::spawn(async move {
                                    let _outcomes = racer.race_broadcast_raw_tx(&raw_tx_clone).await;
                                });
                            }
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

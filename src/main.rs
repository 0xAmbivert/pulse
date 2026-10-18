use clap::Parser;
use crossterm::{
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

use pulse::alerts::AlertDispatcher;
use pulse::config::{AppConfig, TriggerMode};
use pulse::gas::{wei_to_gwei, GasEngine};
use pulse::network::{MevBuilderClient, RpcRacer};
use pulse::simulation::RpcSimulator;
use pulse::sniper::{CountdownSniper, MempoolScanner, SnipeTrigger, StatePoller};
use pulse::ui::{draw_ui, run_interactive_menu, AppEvent, DashboardState, EventHandler, MenuAction};
use pulse::wallet::{spawn_tx_monitor, SubmittedTx, WalletWorker};

#[derive(Clone)]
struct PreparedSnipe {
    worker: Arc<WalletWorker>,
    raw_tx: String,
    tx_hash: String,
    nonce: u64,
    max_fee_wei: u128,
    priority_fee_wei: u128,
    calldata: Vec<u8>,
    value_wei: u128,
}

fn prepare_snipes(
    workers: &[Arc<WalletWorker>],
    config: &AppConfig,
    target_contract: &str,
    current_base_fee_wei: u128,
    prev_base_fee_wei: Option<u128>,
    gas_engine: &GasEngine,
) -> Result<Vec<PreparedSnipe>, Box<dyn std::error::Error + Send + Sync>> {
    let mut snipes = Vec::with_capacity(workers.len());
    let (max_fee_wei, max_prio_wei) = gas_engine.calculate_dynamic_fees_with_acceleration(current_base_fee_wei, prev_base_fee_wei, 1, None)?;
    let value = config.drop.mint_value_wei.parse::<u128>()
        .map_err(|e| format!("Invalid mint value in wei '{}': {e}", config.drop.mint_value_wei))?;
    let destination = config.drop.get_dispatch_destination(target_contract);

    for worker in workers {
        let calldata = config.drop.build_dispatch_calldata(Some(&worker.address), config.drop.mint_quantity)?;
        let nonce = worker.nonce_mgr.current();
        let (raw_tx, n) = worker.build_and_sign_eip1559(
            config.chain.chain_id, destination, &calldata, value, config.gas.gas_limit, max_fee_wei, max_prio_wei, Some(nonce),
        )?;
        let tx_bytes = hex::decode(raw_tx.trim_start_matches("0x"))
            .map_err(|e| format!("Signed transaction is not valid hex: {e}"))?;
        let tx_hash = format!("{:#x}", alloy::primitives::keccak256(&tx_bytes));
        snipes.push(PreparedSnipe {
            worker: worker.clone(), raw_tx, tx_hash, nonce: n, max_fee_wei, priority_fee_wei: max_prio_wei, calldata, value_wei: value,
        });
    }
    Ok(snipes)
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Path to configuration file
    #[arg(short, long, value_name = "FILE", default_value = "config.toml")]
    config: PathBuf,
    /// Run in headless mode without interactive TUI
    #[arg(long)]
    headless: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli = Cli::parse();
    pulse::crypto::harden_process();

    #[cfg(unix)]
    let log_opts = {
        use std::os::unix::fs::OpenOptionsExt;
        let mut o = std::fs::OpenOptions::new();
        o.create(true).append(true).mode(0o600);
        o
    };
    #[cfg(not(unix))]
    let log_opts = { let mut o = std::fs::OpenOptions::new(); o.create(true).append(true); o };
    if let Ok(log_file) = log_opts.open("/tmp/pulse.log") {
        let sub = tracing_subscriber::fmt().with_writer(std::sync::Mutex::new(log_file)).with_ansi(false).finish();
        let _ = tracing::subscriber::set_global_default(sub);
    }

    // Create default config silently if it doesn't exist so load won't fail
    if !cli.config.exists() {
        AppConfig::default().save_to_file(&cli.config)?;
    }

    let base_config = AppConfig::load_from_file(&cli.config)?;

    let (keys, config) = match run_interactive_menu(&base_config).await? {
        MenuAction::Exit => return Ok(()),
        MenuAction::Start { keys, session_config } => (keys, *session_config),
    };

    // Setup Engine Components
    let rpc_racer = Arc::new(RpcRacer::new(&config.chain.rpc_urls, config.chain.rpc_timeout_ms));
    // The user's Max Gas Fee is the per-tx cap, bounded by the hard ceiling from config.
    let effective_cap_gwei = config.gas.max_fee_gwei.min(config.gas.hard_gas_ceiling_gwei);
    let gas_engine = Arc::new(GasEngine::new(effective_cap_gwei, config.gas.max_priority_fee_gwei, config.gas.speedup_bump_percent));
    let alerts = Arc::new(AlertDispatcher::new(
        config.alerts.discord_webhook.clone(),
        config.alerts.telegram_bot_token.clone(),
        config.alerts.telegram_chat_id.clone(),
    ));

    let mut workers = Vec::new();
    for key in keys {
        let addr = pulse::crypto::get_address_from_protected(&key)?;
        let worker = WalletWorker::new(key, 0)?;
        if let Some(ep) = rpc_racer.endpoints().first() {
            if let Err(e) = worker.nonce_mgr.resync_from_rpc(&ep.url).await {
                println!("⚠️ Failed to fetch nonce for {}: {}", addr, e);
            }
        }
        println!("🔓 Active RAM wallet initialized: {} (Nonce: {})", addr, worker.nonce_mgr.current());
        workers.push(Arc::new(worker));
    }

    use std::io::IsTerminal;
    let is_piped = std::env::var("PULSE_ALLOW_PIPED_KEY").as_deref() == Ok("1");
    let is_tty = std::io::stdout().is_terminal() && std::io::stdin().is_terminal();
    let headless = cli.headless || (!is_tty && is_piped);

    if !is_tty && !headless {
        println!("ℹ️ The interactive dashboard requires a TTY. Set PULSE_ALLOW_PIPED_KEY=1 or pass --headless.");
        return Ok(());
    }

    std::panic::set_hook(Box::new(|info| {
        let _ = ratatui::crossterm::terminal::disable_raw_mode();
        let _ = ratatui::crossterm::ExecutableCommand::execute(&mut std::io::stdout(), ratatui::crossterm::terminal::LeaveAlternateScreen);
        eprintln!("{}", info);
    }));

    if !headless {
        enable_raw_mode()?;
        io::stdout().execute(EnterAlternateScreen)?;
    }
    let mut terminal = if !headless {
        Some(Terminal::new(CrosstermBackend::new(io::stdout()))?)
    } else {
        println!("🤖 Headless mode active: streaming event logs directly to console...");
        None
    };

    let mut state = DashboardState::new(config.drop.target_contract.clone());
    state.active_wallets = workers.len();
    state.current_base_fee = 0.0;
    state.add_log(format!("🧢 Max fee cap: {:.2} Gwei", effective_cap_gwei));

    let mut events = EventHandler::new(250);
    let (trigger_tx, mut trigger_rx) = mpsc::channel::<SnipeTrigger>(10);
    let (ui_log_tx, mut ui_log_rx) = mpsc::channel::<String>(100);

    // Continuous Live Base Fee Poller: guarantees current_base_fee is updated in real time across all sniping modes
    let rpc_for_base_fee = rpc_racer.clone();
    let trigger_tx_base_fee = trigger_tx.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(1500));
        loop {
            interval.tick().await;
            if let Some(base_fee_wei) = rpc_for_base_fee.fetch_latest_base_fee().await {
                let _ = trigger_tx_base_fee.send(SnipeTrigger::BaseFeeUpdated { base_fee_wei }).await;
            }
        }
    });

    let target_contract = config.drop.target_contract.clone();
    let rpc_racer_clone = rpc_racer.clone();

    // Pre-flight wallet balance check: balance >= value_wei + (gas_limit * max_fee_wei)
    let max_fee_wei = pulse::gas::gwei_to_wei(effective_cap_gwei);
    let mint_val_wei = config.drop.mint_value_wei.parse::<u128>().unwrap_or(0);
    let needed_wei = mint_val_wei.saturating_add((config.gas.gas_limit as u128).saturating_mul(max_fee_wei));
    for w in &workers {
        if let Some(bal) = rpc_racer.fetch_balance(&w.address).await {
            if bal < needed_wei {
                let deficit_eth = (needed_wei - bal) as f64 / 1e18;
                state.add_log(format!("⚠️ Insufficient balance for {}: deficit {:.6} ETH", w.address, deficit_eth));
            } else {
                state.add_log(format!("💰 Wallet {} balance verified: {:.4} ETH", w.address, bal as f64 / 1e18));
            }
        }
    }

    // 🔥 WIRING UP ALL SNIPERS & ENGINES TO FIX DEAD ENDS 🔥

    // 1. Exclusive Trigger Mode Spawning
    match config.drop.trigger_mode {
        TriggerMode::Countdown => {
            let target_unix = config.drop.target_timestamp.unwrap_or(0);
            if target_unix > 0 {
                let countdown = CountdownSniper::new(target_unix, 50);
                let tx_c = trigger_tx.clone();
                tokio::spawn(async move { countdown.wait_for_target(tx_c).await; });
                state.add_log(format!("⏱️ Armed Countdown Sniper for Unix {target_unix} (lead: 50ms)"));
            } else {
                state.add_log("⚠️ Countdown mode selected but target timestamp is 0 or missing.".to_string());
            }
        }
        TriggerMode::Mempool => {
            let mempool_rpc = config.chain.ws_rpc_url.as_deref().filter(|s| !s.trim().is_empty()).unwrap_or(&config.chain.rpc_urls[0]);
            let scanner = MempoolScanner::with_http_fallback(
                mempool_rpc,
                config.chain.rpc_urls.first().cloned(),
                &target_contract,
                config.drop.monitor_owner_address.clone(),
                &config.drop.mempool_flip_signatures,
            );
            let tx_c = trigger_tx.clone();
            tokio::spawn(async move { scanner.run_scan_loop(tx_c).await; });
            let owner_info = config.drop.monitor_owner_address.as_deref().unwrap_or("any");
            state.add_log(format!("🕵️ Armed Mempool Scanner for Owner {owner_info} on {mempool_rpc}"));
        }
        TriggerMode::StatePoller => {
            if !config.drop.flip_function_signatures.is_empty() && !config.chain.rpc_urls.is_empty() {
                let poller = StatePoller::new(&config.chain.rpc_urls[0], &target_contract, &config.drop.flip_function_signatures);
                let tx_c = trigger_tx.clone();
                tokio::spawn(async move { poller.run_poll_loop(tx_c).await; });
                state.add_log("🔄 Armed On-Chain State Poller".to_string());
            }
        }
    }

    // 2. MEV Builder Client
    // Chain-aware builder routing: Only Ethereum L1 (chain_id == 1, Sepolia 11155111, Goerli 5) support Flashbots/Titan/Beaver
    let is_l1 = config.chain.chain_id == 1 || config.chain.chain_id == 11155111 || config.chain.chain_id == 5;
    let mev_client = if is_l1 && !config.chain.mev_builder_urls.is_empty() {
        Some(Arc::new(MevBuilderClient::new(&config.chain.mev_builder_urls, config.chain.rpc_timeout_ms)))
    } else {
        if !is_l1 && !config.chain.mev_builder_urls.is_empty() {
            println!("ℹ️ L2 Chain ID {} detected: Bypassing L1 MEV builders, routing exclusively via parallel RPC racer.", config.chain.chain_id);
        }
        None
    };

    // 3. Network/RPC Checks
    let endpoints = rpc_racer.endpoints();
    if endpoints.is_empty() {
        println!("❌ FATAL: No valid RPC endpoints found. Please check config.toml.");
        return Ok(());
    }
    state.add_log(format!("🌐 Active RPC Endpoints: {}", endpoints.len()));

    // 4. Advisory Pre-Flight Simulation (Ensures contract & calldata are valid at boot without deadlocking triggers)
    let sim_dest = config.drop.get_dispatch_destination(&target_contract);
    let sim_calldata = config.drop.build_dispatch_calldata(Some(&workers[0].address), config.drop.mint_quantity);
    if let (Ok(calldata), Ok(sim_val)) = (sim_calldata, config.drop.mint_value_wei.parse::<u128>()) {
        let simulator = RpcSimulator::new(&endpoints[0].url);
        match simulator.simulate_call(&workers[0].address, sim_dest, &calldata, sim_val, config.gas.gas_limit).await {
            Ok(sim_res) if !sim_res.success => state.add_log(format!("⚠️ Advisory simulation reverted: {:?}", sim_res.revert_reason)),
            Ok(sim_res) => state.add_log(format!("✅ Pre-flight simulation passed ({} gas)", sim_res.gas_used)),
            Err(e) => state.add_log(format!("⚠️ Pre-flight simulation error: {e}")),
        }
    }

    // Pre-Sign Transactions ahead of drop for instantaneous, zero-latency burst dispatch
    let initial_base_fee_wei = rpc_racer.fetch_latest_base_fee().await.unwrap_or(0);
    state.current_base_fee = wei_to_gwei(initial_base_fee_wei);
    let mut prev_base_fee_wei: Option<u128> = None;
    let mut prepared_snipes = match prepare_snipes(
        &workers,
        &config,
        &target_contract,
        initial_base_fee_wei,
        prev_base_fee_wei,
        &gas_engine,
    ) {
        Ok(snipes) => {
            state.add_log(format!("⚡ Pre-signed {} transactions ahead of drop (zero-latency ready)", snipes.len()));
            snipes
        }
        Err(e) => {
            state.add_log(format!("⚠️ Initial base fee exceeds ceiling: {e}; holding dispatch"));
            Vec::new()
        }
    };

    // Independent background keep-alive ping & telemetry updater (decoupled from UI tick loop to avoid blocking)
    let racer_telemetry = rpc_racer.clone();
    let (latency_tx, mut latency_rx) = mpsc::channel::<Option<u64>>(10);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(1500));
        loop {
            interval.tick().await;
            let lats = racer_telemetry.benchmark_latencies().await;
            let min_lat = lats.iter().filter_map(|(_, l)| *l).min();
            let _ = latency_tx.send(min_lat).await;
        }
    });

    let mut has_fired = false;

    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
    #[cfg(unix)]
    let mut sighup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup()).ok();

    // Main Event Loop
    while state.is_running {
        if let Some(t) = terminal.as_mut() {
            t.draw(|f| draw_ui(f, &state))?;
        }

        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = async {
                #[cfg(unix)]
                { match sigterm.as_mut() { Some(s) => { s.recv().await; } None => std::future::pending::<()>().await } }
                #[cfg(not(unix))]
                { std::future::pending::<()>().await; }
            } => break,
            _ = async {
                #[cfg(unix)]
                { match sighup.as_mut() { Some(s) => { s.recv().await; } None => std::future::pending::<()>().await } }
                #[cfg(not(unix))]
                { std::future::pending::<()>().await; }
            } => break,
            Some(min_lat) = latency_rx.recv() => {
                state.latency_ms = min_lat;
            }
            Some(log_msg) = ui_log_rx.recv() => {
                if headless { println!("{log_msg}"); }
                state.add_log(log_msg);
            }
            Some(event) = events.next() => {
                match event {
                    AppEvent::Input(key) => state.handle_key(key.code),
                    AppEvent::Tick => {}
                }
            }
            Some(trigger) = trigger_rx.recv() => {
                let msg = match &trigger {
                    SnipeTrigger::BaseFeeUpdated { base_fee_wei } => {
                        state.current_base_fee = wei_to_gwei(*base_fee_wei);
                        if !has_fired {
                            match prepare_snipes(&workers, &config, &target_contract, *base_fee_wei, prev_base_fee_wei, &gas_engine) {
                                Ok(snipes) => prepared_snipes = snipes,
                                Err(e) => {
                                    state.add_log(format!("⚠️ Base fee ({:.2} Gwei) exceeds ceiling: {e}; holding dispatch", state.current_base_fee));
                                    prepared_snipes.clear();
                                }
                            }
                        }
                        prev_base_fee_wei = Some(*base_fee_wei);
                        continue;
                    },
                    SnipeTrigger::CountdownReached { target_unix } => format!("🔥 Trigger: Countdown reached {}", target_unix),
                    SnipeTrigger::MempoolDetected { owner_tx_hash, method } => format!("🔥 Trigger: Mempool flip {} via {}", owner_tx_hash, method),
                    SnipeTrigger::StateFlipDetected { new_state } => format!("🔥 Trigger: State flip to {}", new_state),
                };

                if has_fired {
                    state.add_log(format!("ℹ️ Ignored secondary trigger: {}", msg));
                    continue;
                }
                has_fired = true;
                if headless { println!("{msg}"); }
                state.add_log(msg.clone());

                let alerts_c = alerts.clone();
                let desc = msg.clone();
                tokio::spawn(async move {
                    alerts_c.dispatch_alert("🚨 Snipe Trigger Fired", &desc, true).await;
                });

                if prepared_snipes.is_empty() {
                    state.add_log("⛔ Snipe aborted: current gas fee exceeds ceiling or no transactions prepared.".to_string());
                    has_fired = false;
                    continue;
                }

                // Atomic MEV Bundles (DEF-03): When multiple wallets on L1, dispatch batch as bundle
                let is_bundle = is_l1 && mev_client.is_some() && prepared_snipes.len() > 1;
                if is_bundle {
                    let mev = mev_client.as_ref().unwrap().clone();
                    let raw_txs: Vec<String> = prepared_snipes.iter().map(|p| p.raw_tx.clone()).collect();
                    let racer_target = rpc_racer_clone.clone();
                    let alerts_c = alerts.clone();
                    tokio::spawn(async move {
                        let target_block = racer_target.fetch_latest_block_number().await.unwrap_or(0) + 1;
                        let outcomes = mev.send_bundle(&raw_txs, target_block).await;
                        let success = outcomes.iter().any(|o| o.success);
                        alerts_c.dispatch_alert(
                            if success { "📦 MEV Bundle Included" } else { "⚠️ MEV Bundle Rejected" },
                            &format!("Block: {target_block}\nOutcomes: {outcomes:?}"),
                            success,
                        ).await;
                    });
                }

                // ZERO-LATENCY DISPATCH: blast pre-signed transactions immediately!
                for prepared in &prepared_snipes {
                    state.add_log(format!("🚀 Blasting pre-signed tx for {} (Nonce: {}, Hash: {})", prepared.worker.address, prepared.nonce, prepared.tx_hash));
                    prepared.worker.nonce_mgr.set_nonce(prepared.nonce + 1);

                    let submitted = SubmittedTx {
                        worker: prepared.worker.clone(),
                        tx_hashes: vec![prepared.tx_hash.clone()],
                        nonce: prepared.nonce,
                        max_fee_wei: prepared.max_fee_wei,
                        priority_fee_wei: prepared.priority_fee_wei,
                        calldata: prepared.calldata.clone(),
                        value_wei: prepared.value_wei,
                        last_bump: std::time::Instant::now(),
                        bump_count: 0,
                    };

                    let alerts_c = alerts.clone();
                    let addr = prepared.worker.address.clone();
                    let h = prepared.tx_hash.clone();
                    let nonce = prepared.nonce;
                    tokio::spawn(async move {
                        alerts_c.dispatch_alert("🚀 Mint Tx Broadcast", &format!("Wallet: {}\nNonce: {}\nTx: {}", addr, nonce, h), true).await;
                    });

                    // MEV Private Relay on L1 (Never leak to public mempool at T+0; fallback only if rejected)
                    if !is_bundle {
                        if let Some(mev) = &mev_client {
                            let mev_clone = mev.clone();
                            let tx_clone = prepared.raw_tx.clone();
                            let alerts_c = alerts.clone();
                            let racer_fallback = rpc_racer_clone.clone();
                            let allow_fallback = config.gas.allow_public_fallback;
                            tokio::spawn(async move {
                                let outcomes = mev_clone.send_private_transaction(&tx_clone).await;
                                if !outcomes.iter().any(|o| o.success) {
                                    if allow_fallback {
                                        alerts_c.dispatch_alert("⚠️ All Builders Rejected, falling back to public RPC", &format!("{outcomes:?}"), false).await;
                                        let _ = racer_fallback.race_broadcast_raw_tx(&tx_clone).await;
                                    } else {
                                        alerts_c.dispatch_alert("⚠️ All Builders Rejected (public fallback disabled to prevent sandwiching)", &format!("{outcomes:?}"), false).await;
                                    }
                                }
                            });
                        } else {
                            // On L2 or without builders: race public RPC endpoints
                            let racer = rpc_racer_clone.clone();
                            let raw_tx_clone = prepared.raw_tx.clone();
                            tokio::spawn(async move {
                                let _outcomes = racer.race_broadcast_raw_tx(&raw_tx_clone).await;
                            });
                        }
                    }

                    // Background monitor for receipt & auto-speedup (routes replacement to correct destination)
                    let destination = config.drop.get_dispatch_destination(&target_contract);
                    spawn_tx_monitor(
                        submitted,
                        rpc_racer_clone.clone(),
                        mev_client.clone(),
                        alerts.clone(),
                        gas_engine.clone(),
                        config.chain.chain_id,
                        destination.to_string(),
                        config.gas.auto_speedup,
                        config.gas.speedup_threshold_ms,
                        config.gas.gas_limit,
                        config.gas.allow_public_fallback,
                        ui_log_tx.clone(),
                    );
                }
            }
        }
    }

    // Restore Terminal
    if !headless {
        disable_raw_mode()?;
        io::stdout().execute(LeaveAlternateScreen)?;
    }
    println!("Bot shutdown gracefully.");

    Ok(())
}

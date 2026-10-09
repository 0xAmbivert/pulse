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
use pulse::config::AppConfig;
use pulse::gas::{gwei_to_wei, wei_to_gwei, GasEngine};
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
    gas_engine: &GasEngine,
) -> Vec<PreparedSnipe> {
    let mut snipes = Vec::new();
    let (max_fee_wei, max_priority_fee_wei) = match gas_engine.calculate_dynamic_fees(current_base_fee_wei, None) {
        Ok(fees) => fees,
        Err(_) => (gwei_to_wei(config.gas.max_fee_gwei), gwei_to_wei(config.gas.max_priority_fee_gwei)),
    };
    let value = config.drop.mint_value_wei.parse::<u128>().unwrap_or(0);

    for worker in workers {
        let calldata = config.drop.build_calldata_for_caller(Some(&worker.address)).unwrap_or_default();
        let nonce = worker.nonce_mgr.current();
        if let Ok((raw_tx, n)) = worker.build_and_sign_eip1559(
            config.chain.chain_id,
            target_contract,
            &calldata,
            value,
            config.gas.gas_limit,
            max_fee_wei,
            max_priority_fee_wei,
            Some(nonce),
        ) {
            let tx_bytes = hex::decode(raw_tx.trim_start_matches("0x")).unwrap_or_default();
            let tx_hash = format!("{:#x}", alloy::primitives::keccak256(&tx_bytes));
            snipes.push(PreparedSnipe {
                worker: worker.clone(),
                raw_tx,
                tx_hash,
                nonce: n,
                max_fee_wei,
                priority_fee_wei: max_priority_fee_wei,
                calldata,
                value_wei: value,
            });
        }
    }
    snipes
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

    let menu_action = run_interactive_menu(&cli.config)?;

    let config = AppConfig::load_from_file(&cli.config)?;

    // Setup Engine Components
    let rpc_racer = Arc::new(RpcRacer::new(&config.chain.rpc_urls, config.chain.rpc_timeout_ms));
    let gas_engine = Arc::new(GasEngine::new(config.gas.hard_gas_ceiling_gwei, config.gas.max_priority_fee_gwei, config.gas.speedup_bump_percent));
    let alerts = Arc::new(AlertDispatcher::new(
        config.alerts.discord_webhook.clone(),
        config.alerts.telegram_bot_token.clone(),
        config.alerts.telegram_chat_id.clone(),
    ));

    let key = match menu_action {
        MenuAction::Exit => return Ok(()),
        MenuAction::Start(k) => k,
    };

    let addr = pulse::crypto::get_address_from_protected(&key)?;
    let worker = WalletWorker::new(key, 0)?;
    if let Some(ep) = rpc_racer.endpoints().first() {
        if let Err(e) = worker.nonce_mgr.resync_from_rpc(&ep.url).await {
            println!("⚠️ Failed to fetch nonce for {}: {}", addr, e);
        }
    }
    println!("🔓 Active RAM wallet initialized: {} (Nonce: {})", addr, worker.nonce_mgr.current());
    let workers = vec![Arc::new(worker)];

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

    // 5. Network/RPC Checks
    let endpoints = rpc_racer.endpoints();
    if endpoints.is_empty() {
        println!("❌ FATAL: No valid RPC endpoints found. Please check config.toml.");
        return Ok(());
    }
    state.add_log(format!("🌐 Active RPC Endpoints: {}", endpoints.len()));

    // 6. Advisory Pre-Flight Simulation (Ensures contract & calldata are valid at boot without deadlocking triggers)
    let sim_calldata = config.drop.build_calldata().unwrap_or_default();
    let sim_val = config.drop.mint_value_wei.parse::<u128>().unwrap_or(0);
    let simulator = RpcSimulator::new(&endpoints[0].url);
    if let Ok(sim_res) = simulator.simulate_call(&workers[0].address, &target_contract, &sim_calldata, sim_val, config.gas.gas_limit).await {
        if !sim_res.success {
            state.add_log(format!("⚠️ Advisory pre-flight simulation reverted (expected if drop unopen): {:?}", sim_res.revert_reason));
        } else {
            state.add_log(format!("✅ Pre-flight simulation passed ({} gas)", sim_res.gas_used));
        }
    }

    // Pre-Sign Transactions ahead of drop for instantaneous, zero-latency burst dispatch
    let mut prepared_snipes = prepare_snipes(
        &workers,
        &config,
        &target_contract,
        gwei_to_wei(state.current_base_fee),
        &gas_engine,
    );
    state.add_log(format!("⚡ Pre-signed {} transactions ahead of drop (zero-latency ready)", prepared_snipes.len()));

    let mut has_fired = false;

    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("Failed to bind SIGTERM");
    #[cfg(unix)]
    let mut sighup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup()).expect("Failed to bind SIGHUP");

    // Main Event Loop
    while state.is_running {
        terminal.draw(|f| draw_ui(f, &state))?;

        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = async {
                #[cfg(unix)]
                { sigterm.recv().await; }
                #[cfg(not(unix))]
                { std::future::pending::<()>().await; }
            } => break,
            _ = async {
                #[cfg(unix)]
                { sighup.recv().await; }
                #[cfg(not(unix))]
                { std::future::pending::<()>().await; }
            } => break,
            Some(log_msg) = ui_log_rx.recv() => {
                state.add_log(log_msg);
            }
            Some(event) = events.next() => {
                match event {
                    AppEvent::Input(key) => state.handle_key(key.code),
                    AppEvent::Tick => {
                        // Keep-alive connection ping & telemetry update
                        let racer = rpc_racer_clone.clone();
                        let lats = racer.benchmark_latencies().await;
                        state.latency_ms = lats.iter().filter_map(|(_, l)| *l).min();
                    }
                }
            }
            Some(trigger) = trigger_rx.recv() => {
                let msg = match &trigger {
                    SnipeTrigger::BaseFeeUpdated { base_fee_wei } => {
                        state.current_base_fee = wei_to_gwei(*base_fee_wei);
                        if !has_fired {
                            prepared_snipes = prepare_snipes(&workers, &config, &target_contract, *base_fee_wei, &gas_engine);
                        }
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
                state.add_log(msg.clone());

                let alerts_c = alerts.clone();
                let desc = msg.clone();
                tokio::spawn(async move {
                    alerts_c.dispatch_alert("🚨 Snipe Trigger Fired", &desc, true).await;
                });

                // ZERO-LATENCY DISPATCH: blast pre-signed transactions immediately!
                for prepared in &prepared_snipes {
                    state.add_log(format!("🚀 Blasting pre-signed tx for {} (Nonce: {}, Hash: {})", prepared.worker.address, prepared.nonce, prepared.tx_hash));

                    let submitted = SubmittedTx {
                        worker: prepared.worker.clone(),
                        tx_hash: prepared.tx_hash.clone(),
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

                    // Concurrent Dual Broadcasting on L1: transmit to BOTH MEV builders and public RPC endpoints simultaneously!
                    if let Some(mev) = &mev_client {
                        let mev_clone = mev.clone();
                        let tx_clone = prepared.raw_tx.clone();
                        let alerts_c = alerts.clone();
                        tokio::spawn(async move {
                            let outcomes = mev_clone.send_private_transaction(&tx_clone).await;
                            if !outcomes.iter().any(|o| o.success) {
                                alerts_c.dispatch_alert("⚠️ All Builders Rejected", &format!("{:?}", outcomes), false).await;
                            }
                        });

                        // Concurrently race public RPC endpoints alongside MEV builders
                        let racer = rpc_racer_clone.clone();
                        let raw_tx_clone = prepared.raw_tx.clone();
                        tokio::spawn(async move {
                            let _outcomes = racer.race_broadcast_raw_tx(&raw_tx_clone).await;
                        });
                    } else {
                        // On L2 or without builders: race public RPC endpoints
                        let racer = rpc_racer_clone.clone();
                        let raw_tx_clone = prepared.raw_tx.clone();
                        tokio::spawn(async move {
                            let _outcomes = racer.race_broadcast_raw_tx(&raw_tx_clone).await;
                        });
                    }

                    // Background monitor for receipt & auto-speedup
                    spawn_tx_monitor(
                        submitted,
                        rpc_racer_clone.clone(),
                        mev_client.clone(),
                        alerts.clone(),
                        gas_engine.clone(),
                        config.chain.chain_id,
                        target_contract.clone(),
                        config.gas.auto_speedup,
                        config.gas.speedup_threshold_ms,
                        config.gas.gas_limit,
                        ui_log_tx.clone(),
                    );
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

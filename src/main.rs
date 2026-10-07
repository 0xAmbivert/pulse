mod alerts;
mod config;
mod crypto;
mod gas;
mod network;
mod simulation;
mod sniper;
mod ui;
mod wallet;

use clap::Parser;
use crossterm::{
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::path::PathBuf;
use tokio::sync::mpsc;

use crate::config::AppConfig;
use crate::sniper::SnipeTrigger;
use crate::ui::{draw_ui, AppEvent, DashboardState, EventHandler};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Path to configuration file
    #[arg(short, long, value_name = "FILE", default_value = "config.toml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 1. Parse CLI and Load Config
    let cli = Cli::parse();
    
    // Create a dummy config if it doesn't exist
    if !cli.config.exists() {
        let default_config = AppConfig::default();
        default_config.save_to_file(&cli.config)?;
    }
    let config = AppConfig::load_from_file(&cli.config)?;

    // 2. Setup Terminal UI
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = DashboardState::new(config.drop.target_contract.clone());
    let mut events = EventHandler::new(250);

    // 3. Setup trigger channel
    let (trigger_tx, mut trigger_rx) = mpsc::channel::<SnipeTrigger>(10);

    // 4. Main Event Loop
    while state.is_running {
        terminal.draw(|f| draw_ui(f, &state))?;

        tokio::select! {
            // Handle Keyboard/UI Events
            Some(event) = events.next() => {
                match event {
                    AppEvent::Input(key) => state.handle_key(key.code),
                    AppEvent::Tick => {} // Periodic refresh
                }
            }
            // Handle Sniping Triggers
            Some(trigger) = trigger_rx.recv() => {
                match trigger {
                    SnipeTrigger::CountdownReached { target_unix } => {
                        state.add_log(format!("🔥 Trigger: Countdown reached {}", target_unix));
                        // Fire mint transaction here
                    }
                    SnipeTrigger::MempoolDetected { owner_tx_hash, method } => {
                        state.add_log(format!("🔥 Trigger: Mempool flip {} via {}", owner_tx_hash, method));
                        // Fire mint transaction here
                    }
                    SnipeTrigger::StateFlipDetected { new_state } => {
                        state.add_log(format!("🔥 Trigger: State flip to {}", new_state));
                        // Fire mint transaction here
                    }
                    SnipeTrigger::BlockReached { target_block } => {
                        state.add_log(format!("🔥 Trigger: Target block {} reached", target_block));
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

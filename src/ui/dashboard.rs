use crossterm::event::KeyCode;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

pub struct DashboardState {
    pub is_running: bool,
    pub active_wallets: usize,
    pub target_contract: String,
    pub recent_logs: Vec<String>,
    pub current_base_fee: f64,
    pub latency_ms: Option<u64>,
}

impl DashboardState {
    pub fn new(target_contract: String) -> Self {
        Self {
            is_running: true,
            active_wallets: 0,
            target_contract,
            recent_logs: vec!["Bot initialized. Waiting for trigger...".to_string()],
            current_base_fee: 0.0,
            latency_ms: None,
        }
    }

    pub fn handle_key(&mut self, key: KeyCode) {
        match key {
            KeyCode::Char('q') | KeyCode::Esc => self.is_running = false,
            _ => {}
        }
    }

    pub fn add_log(&mut self, msg: String) {
        self.recent_logs.push(msg);
        if self.recent_logs.len() > 10 {
            self.recent_logs.remove(0);
        }
    }
}

pub fn draw_ui(f: &mut Frame, state: &DashboardState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(
            [
                Constraint::Length(3), // Header
                Constraint::Length(5), // Stats
                Constraint::Min(0),    // Logs
            ]
            .as_ref(),
        )
        .split(f.area());

    // Header
    let header = Paragraph::new(format!(
        " Pulse | Target: {} | Press 'q' to quit",
        state.target_contract
    ))
    .style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, chunks[0]);

    // Stats
    let latency_str = state.latency_ms.map_or("N/A".to_string(), |l| format!("{} ms", l));
    let stats_text = format!(
        " Active Wallets: {} | Ping Latency: {}\n Current Base Fee: {:.2} Gwei",
        state.active_wallets, latency_str, state.current_base_fee
    );
    let stats = Paragraph::new(stats_text)
        .style(Style::default().fg(Color::Yellow))
        .block(Block::default().title(" Metrics ").borders(Borders::ALL));
    f.render_widget(stats, chunks[1]);

    // Logs
    let log_items: Vec<ListItem> = state
        .recent_logs
        .iter()
        .map(|log| ListItem::new(log.as_str()).style(Style::default().fg(Color::White)))
        .collect();

    let logs_list = List::new(log_items)
        .block(Block::default().title(" Event Logs ").borders(Borders::ALL));
    f.render_widget(logs_list, chunks[2]);
}

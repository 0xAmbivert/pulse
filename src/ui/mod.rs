pub mod dashboard;
pub mod events;
pub mod menu;

pub use dashboard::{draw_ui, DashboardState};
pub use events::{AppEvent, EventHandler};
pub use menu::run_interactive_menu;

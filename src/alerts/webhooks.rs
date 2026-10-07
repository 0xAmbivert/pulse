use reqwest::Client;
use serde_json::json;
use std::time::Duration;
use tracing::warn;

pub struct AlertDispatcher {
    client: Client,
    discord_webhook: Option<String>,
    telegram_token: Option<String>,
    telegram_chat_id: Option<String>,
}

impl AlertDispatcher {
    pub fn new(
        discord_webhook: Option<String>,
        telegram_token: Option<String>,
        telegram_chat_id: Option<String>,
    ) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_millis(3000))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            discord_webhook,
            telegram_token,
            telegram_chat_id,
        }
    }

    /// Sends a rich alert notification across configured channels.
    pub async fn dispatch_alert(&self, title: &str, message: &str, is_success: bool) {
        if let Some(ref discord_url) = self.discord_webhook {
            let color = if is_success { 0x00FF88 } else { 0xFF3366 };
            let payload = json!({
                "embeds": [{
                    "title": title,
                    "description": message,
                    "color": color,
                    "timestamp": chrono::Utc::now().to_rfc3339()
                }]
            });

            if let Err(e) = self.client.post(discord_url).json(&payload).send().await {
                warn!("Failed to dispatch Discord alert: {e}");
            }
        }

        if let (Some(ref token), Some(ref chat_id)) = (&self.telegram_token, &self.telegram_chat_id) {
            let tg_url = format!("https://api.telegram.org/bot{token}/sendMessage");
            let text = format!("*{title}*\n\n{message}");
            let payload = json!({
                "chat_id": chat_id,
                "text": text,
                "parse_mode": "Markdown"
            });

            if let Err(e) = self.client.post(&tg_url).json(&payload).send().await {
                warn!("Failed to dispatch Telegram alert: {e}");
            }
        }
    }
}

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

        // Allow fallback to environment variables so webhook secrets need not be written to disk config
        let discord_webhook = discord_webhook
            .filter(|s| !s.trim().is_empty())
            .or_else(|| std::env::var("DISCORD_WEBHOOK").ok().filter(|s| !s.trim().is_empty()));
        let telegram_token = telegram_token
            .filter(|s| !s.trim().is_empty())
            .or_else(|| std::env::var("TELEGRAM_BOT_TOKEN").ok().filter(|s| !s.trim().is_empty()));
        let telegram_chat_id = telegram_chat_id
            .filter(|s| !s.trim().is_empty())
            .or_else(|| std::env::var("TELEGRAM_CHAT_ID").ok().filter(|s| !s.trim().is_empty()));

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

            // `without_url()` keeps webhook URLs (which carry secrets) out of logs.
            if let Err(e) = self.client.post(discord_url).json(&payload).send().await {
                warn!("Failed to dispatch Discord alert: {}", e.without_url());
            }
        }

        if let (Some(ref token), Some(ref chat_id)) = (&self.telegram_token, &self.telegram_chat_id) {
            let tg_url = format!("https://api.telegram.org/bot{token}/sendMessage");
            // Plain text (no parse_mode) so user-controlled content cannot break formatting.
            let text = format!("{title}\n\n{message}");
            let payload = json!({
                "chat_id": chat_id,
                "text": text,
            });

            if let Err(e) = self.client.post(&tg_url).json(&payload).send().await {
                warn!("Failed to dispatch Telegram alert: {}", e.without_url());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_string_env_fallback() {
        std::env::set_var("DISCORD_WEBHOOK", "https://discord.com/api/webhooks/test");
        let dispatcher = AlertDispatcher::new(Some("".to_string()), None, None);
        assert_eq!(dispatcher.discord_webhook.as_deref(), Some("https://discord.com/api/webhooks/test"));
        std::env::remove_var("DISCORD_WEBHOOK");
    }
}

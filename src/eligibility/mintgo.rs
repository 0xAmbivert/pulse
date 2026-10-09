use reqwest::Client;
use serde_json::Value;
use std::sync::RwLock;
use std::time::Duration;
use crate::eligibility::types::MintStageInfo;

pub struct MintGoClient {
    client: Client,
    cached_cookie: RwLock<Option<String>>,
}

impl Default for MintGoClient {
    fn default() -> Self {
        Self::new()
    }
}

impl MintGoClient {
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_millis(6000))
            .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            client,
            cached_cookie: RwLock::new(None),
        }
    }

    /// Initializes a browser session with MintGo to receive session cookies (mg_access, mg_vid).
    pub async fn ensure_session(&self) -> Result<String, String> {
        if let Ok(guard) = self.cached_cookie.read() {
            if let Some(ref c) = *guard {
                return Ok(c.clone());
            }
        }

        let url = "https://mintgo.fun/api/session";
        let resp = self.client.post(url)
            .header("Content-Type", "application/json")
            .header("Origin", "https://mintgo.fun")
            .header("Referer", "https://mintgo.fun/")
            .body("{}")
            .send().await
            .map_err(|e| format!("Failed to bootstrap MintGo session: {e}"))?;

        let mut cookies = Vec::new();
        for val in resp.headers().get_all("set-cookie") {
            if let Ok(s) = val.to_str() {
                let part = s.split(';').next().unwrap_or("").trim();
                if !part.is_empty() {
                    cookies.push(part.to_string());
                }
            }
        }

        let cookie_str = cookies.join("; ");
        if let Ok(mut guard) = self.cached_cookie.write() {
            *guard = Some(cookie_str.clone());
        }
        Ok(cookie_str)
    }

    /// Fetches collection intelligence and mint stages from MintGo.
    pub async fn fetch_collection_intel(&self, chain: &str, contract: &str) -> Result<Value, String> {
        let cookie = self.ensure_session().await.unwrap_or_default();
        let url = format!("https://mintgo.fun/api/collection/{contract}?chain={chain}");
        let mut req = self.client.get(&url).header("Referer", "https://mintgo.fun/");
        if !cookie.is_empty() {
            req = req.header("Cookie", cookie);
        }

        let resp = req.send().await
            .map_err(|e| format!("MintGo API error: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("MintGo returned HTTP {}", resp.status()));
        }

        let body: Value = resp.json().await
            .map_err(|e| format!("Failed to parse MintGo JSON: {e}"))?;
        Ok(body)
    }

    /// Checks live eligibility and transaction generation from MintGo.
    pub async fn check_mint_tx(&self, chain: &str, contract: &str, wallet: &str, quantity: u64) -> Result<Value, String> {
        let cookie = self.ensure_session().await.unwrap_or_default();
        let url = format!("https://mintgo.fun/api/mint-tx/{contract}?quantity={quantity}&from={wallet}&chain={chain}");
        let mut req = self.client.get(&url).header("Referer", "https://mintgo.fun/");
        if !cookie.is_empty() {
            req = req.header("Cookie", cookie);
        }

        let resp = req.send().await
            .map_err(|e| format!("MintGo mint-tx request error: {e}"))?;

        let body: Value = resp.json().await
            .map_err(|e| format!("Failed to parse MintGo mint-tx JSON: {e}"))?;
        Ok(body)
    }

    /// Parses mint stages from MintGo collection response.
    pub fn parse_stages(data: &Value) -> Vec<MintStageInfo> {
        let mut stages = Vec::new();
        if let Some(arr) = data.get("mintStages").and_then(|s| s.as_array()) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            for s in arr {
                let id = s.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let stage_type = s.get("type").and_then(|v| v.as_str()).unwrap_or("public_sale").to_string();
                let label = s.get("label").and_then(|v| v.as_str()).unwrap_or("Mint").to_string();
                let price_wei_str = s.get("priceWei").and_then(|v| v.as_str()).unwrap_or("0");
                let price_wei = price_wei_str.parse::<u128>().unwrap_or(0);
                let price_eth = price_wei as f64 / 1e18;

                let start_time_unix = s.get("startTime").and_then(|v| v.as_str()).and_then(parse_rfc3339_to_unix);
                let end_time_unix = s.get("endTime").and_then(|v| v.as_str()).and_then(parse_rfc3339_to_unix);
                let max_per_wallet = s.get("maxPerWallet").and_then(|v| v.as_u64());

                let is_active_now = match (start_time_unix, end_time_unix) {
                    (Some(st), Some(et)) => now >= st && now <= et,
                    (Some(st), None) => now >= st,
                    (None, Some(et)) => now <= et,
                    (None, None) => true,
                };

                stages.push(MintStageInfo {
                    id,
                    stage_type,
                    label,
                    price_wei,
                    price_eth,
                    start_time_unix,
                    end_time_unix,
                    max_per_wallet,
                    is_active_now,
                });
            }
        }
        stages
    }
}

fn parse_rfc3339_to_unix(date_str: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(date_str)
        .ok()
        .map(|dt| dt.timestamp() as u64)
}

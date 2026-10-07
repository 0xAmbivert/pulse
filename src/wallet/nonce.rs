use reqwest::Client;
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub struct NonceManager {
    address: String,
    current_nonce: AtomicU64,
}

impl NonceManager {
    pub fn new(address: &str, initial_nonce: u64) -> Self {
        Self {
            address: address.to_string(),
            current_nonce: AtomicU64::new(initial_nonce),
        }
    }

    /// Returns the current nonce and increments it atomically for immediate subsequent use.
    pub fn get_and_increment(&self) -> u64 {
        self.current_nonce.fetch_add(1, Ordering::SeqCst)
    }

    /// Reads the current cached nonce without incrementing.
    pub fn current(&self) -> u64 {
        self.current_nonce.load(Ordering::SeqCst)
    }

    /// Sets the nonce explicitly.
    pub fn set_nonce(&self, nonce: u64) {
        self.current_nonce.store(nonce, Ordering::SeqCst);
    }

    /// Resynchronizes the nonce by querying `eth_getTransactionCount` with tag "pending" from RPC.
    pub async fn resync_from_rpc(&self, rpc_url: &str) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        let client = Client::builder()
            .timeout(Duration::from_millis(3000))
            .build()?;

        let payload = json!({
            "jsonrpc": "2.0",
            "method": "eth_getTransactionCount",
            "params": [self.address, "pending"],
            "id": 1
        });

        let resp = client.post(rpc_url).json(&payload).send().await?;
        let body: serde_json::Value = resp.json().await?;

        if let Some(result_hex) = body.get("result").and_then(|r| r.as_str()) {
            let clean_hex = result_hex.trim_start_matches("0x");
            let nonce = u64::from_str_radix(clean_hex, 16)?;
            self.set_nonce(nonce);
            Ok(nonce)
        } else {
            Err("Failed to parse nonce from RPC response".into())
        }
    }
}

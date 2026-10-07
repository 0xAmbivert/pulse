use super::countdown::SnipeTrigger;
use alloy::primitives::keccak256;
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::info;

pub struct StatePoller {
    rpc_url: String,
    target_contract: String,
    calldata_hex: String,
}

impl StatePoller {
    pub fn new(rpc_url: &str, target_contract: &str, state_check_func_sig: &str) -> Self {
        let hash = keccak256(state_check_func_sig.as_bytes());
        let selector_hex = format!("0x{}", hex::encode(&hash[0..4]));

        Self {
            rpc_url: rpc_url.to_string(),
            target_contract: target_contract.to_string(),
            calldata_hex: selector_hex,
        }
    }

    /// Continuously polls the view function of the target contract.
    /// Fires when the returned boolean turns true (i.e. not all zeros).
    pub async fn run_poll_loop(&self, trigger_tx: mpsc::Sender<SnipeTrigger>) {
        let client = Client::builder()
            .timeout(Duration::from_millis(1000))
            .build()
            .unwrap_or_else(|_| Client::new());

        let mut interval = tokio::time::interval(Duration::from_millis(150));

        loop {
            interval.tick().await;

            let payload = json!({
                "jsonrpc": "2.0",
                "method": "eth_call",
                "params": [{
                    "to": self.target_contract,
                    "data": self.calldata_hex
                }, "latest"],
                "id": 1
            });

            if let Ok(resp) = client.post(&self.rpc_url).json(&payload).send().await {
                if let Ok(body) = resp.json::<Value>().await {
                    if let Some(result_hex) = body.get("result").and_then(|r| r.as_str()) {
                        let clean = result_hex.trim_start_matches("0x");
                        // If it's a bool, true is represented by ending in 1
                        if clean.ends_with('1') {
                            info!("StatePoller detected sale is now active on contract: {}", self.target_contract);
                            let _ = trigger_tx.send(SnipeTrigger::StateFlipDetected { new_state: true }).await;
                            return;
                        }
                    }
                }
            }
        }
    }
}

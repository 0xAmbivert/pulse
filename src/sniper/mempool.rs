use super::countdown::SnipeTrigger;
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{info, warn};

pub struct MempoolScanner {
    rpc_url: String,
    target_contract: String,
    owner_address: Option<String>,
    selectors: Vec<[u8; 4]>,
}

impl MempoolScanner {
    pub fn new(
        rpc_url: &str,
        target_contract: &str,
        owner_address: Option<String>,
        signature_names: &[String],
    ) -> Self {
        // Pre-compute 4-byte selectors for quick byte matching
        let selectors: Vec<[u8; 4]> = signature_names
            .iter()
            .map(|sig| {
                use alloy::primitives::keccak256;
                let hash = keccak256(sig.as_bytes());
                let mut sel = [0u8; 4];
                sel.copy_from_slice(&hash[0..4]);
                sel
            })
            .collect();

        Self {
            rpc_url: rpc_url.to_string(),
            target_contract: target_contract.to_lowercase(),
            owner_address: owner_address.map(|a| a.to_lowercase()),
            selectors,
        }
    }

    /// Checks if a transaction input calldata matches any known flip function 4-byte selector.
    pub fn matches_selector(&self, input_hex: &str) -> bool {
        let clean = input_hex.trim_start_matches("0x");
        if clean.len() < 8 {
            return false;
        }

        if let Ok(bytes) = hex::decode(&clean[0..8]) {
            if bytes.len() == 4 {
                return self.selectors.iter().any(|s| s == bytes.as_slice());
            }
        }
        false
    }

    /// Monitors pending transactions from the node's txpool/pending status.
    pub async fn run_scan_loop(&self, trigger_tx: mpsc::Sender<SnipeTrigger>) {
        let client = Client::builder()
            .timeout(Duration::from_millis(1500))
            .build()
            .unwrap_or_else(|_| Client::new());

        let mut interval = tokio::time::interval(Duration::from_millis(100));

        loop {
            interval.tick().await;

            let payload = json!({
                "jsonrpc": "2.0",
                "method": "txpool_content",
                "params": [],
                "id": 1
            });

            if let Ok(resp) = client.post(&self.rpc_url).json(&payload).send().await {
                if let Ok(body) = resp.json::<Value>().await {
                    if let Some(pending) = body.get("result").and_then(|r| r.get("pending")) {
                        if let Some(pending_map) = pending.as_object() {
                            for (sender_addr, nonces_map) in pending_map {
                                let sender_lower = sender_addr.to_lowercase();
                                let is_owner_match = self.owner_address.as_ref().map_or(false, |o| *o == sender_lower);

                                if let Some(nonces) = nonces_map.as_object() {
                                    for (_nonce_key, tx_obj) in nonces {
                                        let to_addr = tx_obj.get("to").and_then(|t| t.as_str()).unwrap_or("").to_lowercase();
                                        let input_data = tx_obj.get("input").and_then(|i| i.as_str()).unwrap_or("");

                                        let target_matches = to_addr == self.target_contract;
                                        let selector_matches = self.matches_selector(input_data);

                                        if (is_owner_match && target_matches) || (target_matches && selector_matches) {
                                            let hash = tx_obj.get("hash").and_then(|h| h.as_str()).unwrap_or("unknown").to_string();
                                            info!(
                                                "Mempool sniper detected matching trigger tx: {} from {} to {}",
                                                hash, sender_lower, to_addr
                                            );
                                            let _ = trigger_tx.send(SnipeTrigger::MempoolDetected {
                                                owner_tx_hash: hash,
                                                method: input_data.chars().take(10).collect(),
                                            }).await;
                                            return;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

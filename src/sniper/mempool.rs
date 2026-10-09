use super::countdown::SnipeTrigger;
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::info;

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
        if self.rpc_url.starts_with("ws://") || self.rpc_url.starts_with("wss://") {
            info!("Mempool scanner connecting via WebSocket streaming: {}", self.rpc_url);
            self.run_ws_loop(trigger_tx).await;
        } else {
            info!("Mempool scanner running via HTTP polling (500ms): {}", self.rpc_url);
            self.run_http_loop(trigger_tx).await;
        }
    }

    async fn run_ws_loop(&self, trigger_tx: mpsc::Sender<SnipeTrigger>) {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        let mut backoff_ms = 1000u64;
        let http_url = if self.rpc_url.starts_with("wss://") {
            self.rpc_url.replacen("wss://", "https://", 1)
        } else {
            self.rpc_url.replacen("ws://", "http://", 1)
        };

        loop {
            match tokio_tungstenite::connect_async(&self.rpc_url).await {
                Ok((ws_stream, _)) => {
                    backoff_ms = 1000;
                    let (mut write, mut read) = ws_stream.split();
                    let sub_req = json!({
                        "jsonrpc": "2.0",
                        "id": 1,
                        "method": "eth_subscribe",
                        "params": ["newPendingTransactions"]
                    });
                    if write.send(Message::Text(sub_req.to_string().into())).await.is_err() {
                        tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                        continue;
                    }

                    let client = Client::builder()
                        .timeout(Duration::from_millis(2000))
                        .build()
                        .unwrap_or_else(|_| Client::new());

                    while let Some(msg_res) = read.next().await {
                        match msg_res {
                            Ok(Message::Text(text)) => {
                                if let Ok(v) = serde_json::from_str::<Value>(&text) {
                                    if let Some(tx_hash) = v.get("params")
                                        .and_then(|p| p.get("result"))
                                        .and_then(|r| r.as_str())
                                    {
                                        let get_tx = json!({
                                            "jsonrpc": "2.0",
                                            "id": 1,
                                            "method": "eth_getTransactionByHash",
                                            "params": [tx_hash]
                                        });
                                        if let Ok(resp) = client.post(&http_url).json(&get_tx).send().await {
                                            if let Ok(body) = resp.json::<Value>().await {
                                                if let Some(tx_obj) = body.get("result") {
                                                    let sender_addr = tx_obj.get("from").and_then(|f| f.as_str()).unwrap_or("").to_lowercase();
                                                    let to_addr = tx_obj.get("to").and_then(|t| t.as_str()).unwrap_or("").to_lowercase();
                                                    let input_data = tx_obj.get("input").and_then(|i| i.as_str()).unwrap_or("");

                                                    let is_owner_match = self.owner_address.as_ref().is_none_or(|o| *o == sender_addr);
                                                    let target_matches = to_addr == self.target_contract;
                                                    let selector_matches = self.matches_selector(input_data);

                                                    if is_owner_match && target_matches && selector_matches {
                                                        info!(
                                                            "WS Mempool sniper detected matching trigger tx: {} from {} to {}",
                                                            tx_hash, sender_addr, to_addr
                                                        );
                                                        let _ = trigger_tx.send(SnipeTrigger::MempoolDetected {
                                                            owner_tx_hash: tx_hash.to_string(),
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
                            Ok(Message::Close(_)) | Err(_) => {
                                break;
                            }
                            _ => {}
                        }
                    }
                }
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                    backoff_ms = (backoff_ms * 2).min(10000);
                }
            }
        }
    }

    async fn run_http_loop(&self, trigger_tx: mpsc::Sender<SnipeTrigger>) {
        let client = Client::builder()
            .timeout(Duration::from_millis(2000))
            .build()
            .unwrap_or_else(|_| Client::new());

        let mut interval = tokio::time::interval(Duration::from_millis(500));

        loop {
            interval.tick().await;

            let payload = json!({
                "jsonrpc": "2.0",
                "method": "eth_getBlockByNumber",
                "params": ["pending", true],
                "id": 1
            });

            if let Ok(resp) = client.post(&self.rpc_url).json(&payload).send().await {
                if let Ok(body) = resp.json::<Value>().await {
                    if let Some(result) = body.get("result") {
                        if let Some(base_fee_hex) = result.get("baseFeePerGas").and_then(|b| b.as_str()) {
                            if let Ok(base_fee_wei) = u128::from_str_radix(base_fee_hex.trim_start_matches("0x"), 16) {
                                let _ = trigger_tx.send(SnipeTrigger::BaseFeeUpdated { base_fee_wei }).await;
                            }
                        }
                        if let Some(transactions) = result.get("transactions").and_then(|t| t.as_array()) {
                            for tx_obj in transactions {
                                let sender_addr = tx_obj.get("from").and_then(|f| f.as_str()).unwrap_or("").to_lowercase();
                                let to_addr = tx_obj.get("to").and_then(|t| t.as_str()).unwrap_or("").to_lowercase();
                                let input_data = tx_obj.get("input").and_then(|i| i.as_str()).unwrap_or("");

                                let is_owner_match = self.owner_address.as_ref().is_none_or(|o| *o == sender_addr);
                                let target_matches = to_addr == self.target_contract;
                                let selector_matches = self.matches_selector(input_data);

                                if is_owner_match && target_matches && selector_matches {
                                    let hash = tx_obj.get("hash").and_then(|h| h.as_str()).unwrap_or("unknown").to_string();
                                    info!(
                                        "HTTP Mempool sniper detected matching trigger tx: {} from {} to {}",
                                        hash, sender_addr, to_addr
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mempool_matches_selector() {
        let sigs = vec!["isPublicSaleActive()".to_string(), "saleIsActive()".to_string()];
        let scanner = MempoolScanner::new("http://localhost:8545", "0x0000000000000000000000000000000000000000", None, &sigs);

        let hash = alloy::primitives::keccak256("isPublicSaleActive()".as_bytes());
        let selector_hex = format!("0x{}", hex::encode(&hash[0..4]));
        assert!(scanner.matches_selector(&selector_hex));

        assert!(!scanner.matches_selector("0xdeadbeef"));
    }
}

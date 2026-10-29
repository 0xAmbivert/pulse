use super::countdown::SnipeTrigger;
use reqwest::Client;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Semaphore};
use tracing::info;

pub struct MempoolScanner {
    rpc_url: String,
    http_fallback_url: Option<String>,
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
        Self::with_http_fallback(rpc_url, None, target_contract, owner_address, signature_names)
    }

    pub fn with_http_fallback(
        rpc_url: &str,
        http_fallback_url: Option<String>,
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
            http_fallback_url,
            target_contract: target_contract.to_lowercase(),
            owner_address: owner_address.map(|a| a.to_lowercase()),
            selectors,
        }
    }

    /// Checks if a transaction input calldata matches any known flip function 4-byte selector.
    #[inline]
    pub fn matches_selector(&self, input_hex: &str) -> bool {
        selector_matches(&self.selectors, input_hex)
    }
}

#[inline]
fn selector_matches(selectors: &[[u8; 4]], input_hex: &str) -> bool {
    let clean = input_hex.trim_start_matches("0x");
    if clean.len() < 8 {
        return false;
    }
    let mut buf = [0u8; 4];
    if hex::decode_to_slice(&clean[0..8], &mut buf).is_ok() {
        return selectors.contains(&buf);
    }
    false
}

#[inline]
fn matches_tx_object(
    tx_obj: &Value,
    target_contract: &str,
    owner_address: Option<&str>,
    selectors: &[[u8; 4]],
) -> Option<(String, String)> {
    // Check `to` first: 99.99% of pending transactions target other contracts.
    let to_addr = tx_obj.get("to").and_then(|t| t.as_str())?;
    if !to_addr.eq_ignore_ascii_case(target_contract) {
        return None;
    }
    let sender_addr = tx_obj.get("from").and_then(|f| f.as_str()).unwrap_or("");
    if let Some(owner) = owner_address {
        if !sender_addr.eq_ignore_ascii_case(owner) {
            return None;
        }
    }
    let input_data = tx_obj.get("input").and_then(|i| i.as_str()).unwrap_or("");
    if !selector_matches(selectors, input_data) {
        return None;
    }
    let hash = tx_obj
        .get("hash")
        .and_then(|h| h.as_str())
        .unwrap_or("unknown")
        .to_string();
    let method: String = input_data.chars().take(10).collect();
    Some((hash, method))
}

impl MempoolScanner {

    /// Monitors pending transactions from the node's txpool/pending status.
    pub async fn run_scan_loop(&self, trigger_tx: mpsc::Sender<SnipeTrigger>) {
        if self.owner_address.is_none() {
            tracing::warn!("Mempool scanner has no owner filter: any sender calling a flip selector on the target will trigger.");
        }
        if self.rpc_url.starts_with("ws://") || self.rpc_url.starts_with("wss://") {
            info!("Mempool scanner connecting via WebSocket streaming: {}", self.rpc_url);
            self.run_ws_loop(trigger_tx).await;
        } else {
            info!("Mempool scanner running via HTTP polling (500ms): {}", self.rpc_url);
            self.run_http_loop(trigger_tx).await;
        }
    }

    async fn run_ws_loop(&self, trigger_tx: mpsc::Sender<SnipeTrigger>) {
        use futures::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        let mut backoff_ms = 1000u64;
        let mut is_reconnect = false;
        let http_url = self.http_fallback_url.clone().unwrap_or_else(|| {
            if self.rpc_url.starts_with("wss://") {
                self.rpc_url.replacen("wss://", "https://", 1)
            } else {
                self.rpc_url.replacen("ws://", "http://", 1)
            }
        });

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

                    // Also subscribe to newHeads to stream real-time baseFeePerGas updates
                    let sub_heads = json!({
                        "jsonrpc": "2.0",
                        "id": 2,
                        "method": "eth_subscribe",
                        "params": ["newHeads"]
                    });
                    let _ = write.send(Message::Text(sub_heads.to_string().into())).await;

                    let client = Client::builder()
                        .timeout(Duration::from_millis(2000))
                        .pool_max_idle_per_host(32)
                        .tcp_nodelay(true)
                        .build()
                        .unwrap_or_else(|_| Client::new());

                    if is_reconnect {
                        info!("WS Mempool reconnected. Executing pending block catch-up for missed transactions...");
                        self.catch_up_pending(&client, &http_url, &trigger_tx).await;
                    }
                    is_reconnect = true;

                    let sem = Arc::new(Semaphore::new(16));

                    while let Some(msg_res) = read.next().await {
                        match msg_res {
                            Ok(Message::Text(text)) => {
                                if let Ok(v) = serde_json::from_str::<Value>(&text) {
                                    if let Some(params_res) = v.get("params").and_then(|p| p.get("result")) {
                                        // Case 1: newPendingTransactions stream returns string tx_hash
                                        if let Some(tx_hash) = params_res.as_str() {
                                            // Bound in-flight lookups: wait for a permit before spawning.
                                            let permit = match sem.clone().acquire_owned().await {
                                                Ok(p) => p,
                                                Err(_) => break,
                                            };
                                            let client_c = client.clone();
                                            let http_url_c = http_url.clone();
                                            let tx_hash_str = tx_hash.to_string();
                                            let trigger_tx_c = trigger_tx.clone();
                                            let selectors_c = self.selectors.clone();
                                            let owner_addr_c = self.owner_address.clone();
                                            let target_contract_c = self.target_contract.clone();

                                            tokio::spawn(async move {
                                                let _permit = permit;
                                                let get_tx = json!({
                                                    "jsonrpc": "2.0",
                                                    "id": 1,
                                                    "method": "eth_getTransactionByHash",
                                                    "params": [tx_hash_str]
                                                });
                                                if let Ok(resp) = client_c.post(&http_url_c).json(&get_tx).send().await {
                                                    if let Ok(body) = resp.json::<Value>().await {
                                                        if let Some(tx_obj) = body.get("result") {
                                                            if let Some((_, method)) = matches_tx_object(
                                                                tx_obj,
                                                                &target_contract_c,
                                                                owner_addr_c.as_deref(),
                                                                &selectors_c,
                                                            ) {
                                                                info!(
                                                                    "WS Mempool sniper detected matching trigger tx: {}",
                                                                    tx_hash_str
                                                                );
                                                                let _ = trigger_tx_c.send(SnipeTrigger::MempoolDetected {
                                                                    owner_tx_hash: tx_hash_str,
                                                                    method,
                                                                }).await;
                                                            }
                                                        }
                                                    }
                                                }
                                            });
                                        } else if let Some(base_fee_hex) = params_res.get("baseFeePerGas").and_then(|b| b.as_str()) {
                                            // Case 2: newHeads stream returns block header object with baseFeePerGas
                                            if let Ok(base_fee_wei) = u128::from_str_radix(base_fee_hex.trim_start_matches("0x"), 16) {
                                                let _ = trigger_tx.send(SnipeTrigger::BaseFeeUpdated { base_fee_wei }).await;
                                            }
                                        } else if params_res.is_object() {
                                            // Case 3: full pending transaction object streamed directly
                                            if let Some((hash, method)) = matches_tx_object(
                                                params_res,
                                                &self.target_contract,
                                                self.owner_address.as_deref(),
                                                &self.selectors,
                                            ) {
                                                let _ = trigger_tx.send(SnipeTrigger::MempoolDetected {
                                                    owner_tx_hash: hash,
                                                    method,
                                                }).await;
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

    async fn catch_up_pending(&self, client: &Client, http_url: &str, trigger_tx: &mpsc::Sender<SnipeTrigger>) {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": "eth_getBlockByNumber",
            "params": ["pending", true],
            "id": 999
        });

        if let Ok(resp) = client.post(http_url).json(&payload).send().await {
            if let Ok(body) = resp.json::<Value>().await {
                if let Some(result) = body.get("result") {
                    if let Some(base_fee_hex) = result.get("baseFeePerGas").and_then(|b| b.as_str()) {
                        if let Ok(base_fee_wei) = u128::from_str_radix(base_fee_hex.trim_start_matches("0x"), 16) {
                            let _ = trigger_tx.send(SnipeTrigger::BaseFeeUpdated { base_fee_wei }).await;
                        }
                    }
                    if let Some(transactions) = result.get("transactions").and_then(|t| t.as_array()) {
                        for tx_obj in transactions {
                            if let Some((hash, method)) = matches_tx_object(
                                tx_obj,
                                &self.target_contract,
                                self.owner_address.as_deref(),
                                &self.selectors,
                            ) {
                                info!(
                                    "WS Mempool reconnect catch-up detected matching trigger tx: {}",
                                    hash
                                );
                                let _ = trigger_tx.send(SnipeTrigger::MempoolDetected {
                                    owner_tx_hash: hash,
                                    method,
                                }).await;
                                return;
                            }
                        }
                    }
                }
            }
        }
    }

    async fn run_http_loop(&self, trigger_tx: mpsc::Sender<SnipeTrigger>) {
        let client = Client::builder()
            .timeout(Duration::from_millis(2000))
            .pool_max_idle_per_host(16)
            .tcp_nodelay(true)
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
                                if let Some((hash, method)) = matches_tx_object(
                                    tx_obj,
                                    &self.target_contract,
                                    self.owner_address.as_deref(),
                                    &self.selectors,
                                ) {
                                    info!(
                                        "HTTP Mempool sniper detected matching trigger tx: {}",
                                        hash
                                    );
                                    let _ = trigger_tx.send(SnipeTrigger::MempoolDetected {
                                        owner_tx_hash: hash,
                                        method,
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
        let sigs = vec!["setPublicSaleActive(bool)".to_string(), "flipSaleState()".to_string()];
        let scanner = MempoolScanner::new("http://localhost:8545", "0x0000000000000000000000000000000000000000", None, &sigs);

        let hash = alloy::primitives::keccak256("setPublicSaleActive(bool)".as_bytes());
        let selector_hex = format!("0x{}", hex::encode(&hash[0..4]));
        assert!(scanner.matches_selector(&selector_hex));

        assert!(!scanner.matches_selector("0xdeadbeef"));
    }
}

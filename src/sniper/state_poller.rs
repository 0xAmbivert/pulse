use super::countdown::SnipeTrigger;
use alloy::primitives::keccak256;
use futures::stream::{FuturesUnordered, StreamExt};
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::info;

pub struct StatePoller {
    rpc_urls: Vec<String>,
    target_contract: String,
    calldata_hex: Vec<String>,
}

/// Checks if a hex-encoded 32-byte ABI return word equals `true` (`1`) with zero heap allocations.
#[inline]
pub fn is_abi_true(result_hex: &str) -> bool {
    let clean = result_hex.trim().trim_start_matches("0x");
    if clean.len() != 64 {
        return false;
    }
    let bytes = clean.as_bytes();
    bytes[..62].iter().all(|&b| b == b'0') && bytes[62] == b'0' && bytes[63] == b'1'
}

impl StatePoller {
    /// Polls every signature on the given RPC; the first one returning true fires the trigger.
    pub fn new(rpc_url: &str, target_contract: &str, state_check_func_sigs: &[String]) -> Self {
        Self::with_pool(&[rpc_url.to_string()], target_contract, state_check_func_sigs)
    }

    /// Polls every signature across all HTTP RPC endpoints concurrently.
    pub fn with_pool(
        rpc_urls: &[String],
        target_contract: &str,
        state_check_func_sigs: &[String],
    ) -> Self {
        let calldata_hex = state_check_func_sigs
            .iter()
            .map(|sig| format!("0x{}", hex::encode(&keccak256(sig.as_bytes())[0..4])))
            .collect();

        let http_urls: Vec<String> = rpc_urls
            .iter()
            .filter(|u| !u.starts_with("ws://") && !u.starts_with("wss://") && !u.trim().is_empty())
            .cloned()
            .collect();

        Self {
            rpc_urls: http_urls,
            target_contract: target_contract.to_string(),
            calldata_hex,
        }
    }

    /// Continuously polls the view functions of the target contract concurrently across endpoints.
    /// Fires when the ABI-encoded bool return value equals 1 (the full 32-byte word).
    pub async fn run_poll_loop(&self, trigger_tx: mpsc::Sender<SnipeTrigger>) {
        if self.rpc_urls.is_empty() || self.calldata_hex.is_empty() {
            return;
        }

        let client = Client::builder()
            .timeout(Duration::from_millis(1000))
            .pool_max_idle_per_host(16)
            .tcp_nodelay(true)
            .build()
            .unwrap_or_else(|_| Client::new());

        let mut interval = tokio::time::interval(Duration::from_millis(150));

        loop {
            interval.tick().await;

            let mut tasks = FuturesUnordered::new();
            let mut aborts = Vec::with_capacity(self.rpc_urls.len() * self.calldata_hex.len());

            for data in &self.calldata_hex {
                let payload = json!({
                    "jsonrpc": "2.0",
                    "method": "eth_call",
                    "params": [{
                        "to": self.target_contract,
                        "data": data
                    }, "latest"],
                    "id": 1
                });

                for url in &self.rpc_urls {
                    let client_c = client.clone();
                    let url_c = url.clone();
                    let payload_c = payload.clone();
                    let handle = tokio::spawn(async move {
                        if let Ok(resp) = client_c.post(&url_c).json(&payload_c).send().await {
                            if let Ok(body) = resp.json::<Value>().await {
                                if let Some(result_hex) = body.get("result").and_then(|r| r.as_str()) {
                                    if is_abi_true(result_hex) {
                                        return true;
                                    }
                                }
                            }
                        }
                        false
                    });
                    aborts.push(handle.abort_handle());
                    tasks.push(handle);
                }
            }

            while let Some(res) = tasks.next().await {
                if matches!(res, Ok(true)) {
                    for h in aborts {
                        h.abort();
                    }
                    info!("StatePoller detected sale is now active on contract: {}", self.target_contract);
                    let _ = trigger_tx.send(SnipeTrigger::StateFlipDetected { new_state: true }).await;
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_abi_true_zero_alloc() {
        let mut word_true = "0x".to_string();
        word_true.push_str(&"0".repeat(63));
        word_true.push('1');
        assert!(is_abi_true(&word_true));

        let word_false = format!("0x{}", "0".repeat(64));
        assert!(!is_abi_true(&word_false));

        // Non-bool (e.g., 2) must be rejected
        let mut word_two = "0x".to_string();
        word_two.push_str(&"0".repeat(63));
        word_two.push('2');
        assert!(!is_abi_true(&word_two));
        assert!(!is_abi_true("0x01"));
    }
}

use reqwest::Client;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct RpcEndpoint {
    pub url: String,
    pub is_healthy: bool,
}

#[derive(Debug, Clone)]
pub struct BroadcastOutcome {
    pub rpc_url: String,
    pub duration_ms: u64,
    pub success: bool,
    pub tx_hash: Option<String>,
    pub error: Option<String>,
}

pub struct RpcRacer {
    client: Client,
    endpoints: Vec<RpcEndpoint>,
}

impl RpcRacer {
    pub fn new(urls: &[String], timeout_ms: u64) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(32)
            .tcp_nodelay(true)
            .build()
            .unwrap_or_else(|_| Client::new());

        let endpoints = urls
            .iter()
            .map(|url| RpcEndpoint {
                url: url.clone(),
                is_healthy: true,
            })
            .collect();

        Self {
            client,
            endpoints,
        }
    }

    /// Pings all configured RPC endpoints concurrently to evaluate response times.
    pub async fn benchmark_latencies(&mut self) -> Vec<(String, Option<u64>)> {
        let mut tasks = Vec::new();

        for endpoint in &self.endpoints {
            let client = self.client.clone();
            let url = endpoint.url.clone();
            tasks.push(tokio::spawn(async move {
                let payload = json!({
                    "jsonrpc": "2.0",
                    "method": "eth_blockNumber",
                    "params": [],
                    "id": 1
                });

                let start = Instant::now();
                match client.post(&url).json(&payload).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        let elapsed = start.elapsed().as_millis() as u64;
                        (url, Some(elapsed))
                    }
                    _ => (url, None),
                }
            }));
        }

        let mut results = Vec::new();
        for task in tasks {
            if let Ok(res) = task.await {
                // Update local status
                if let Some(ep) = self.endpoints.iter_mut().find(|e| e.url == res.0) {
                    ep.is_healthy = res.1.is_some();
                }
                results.push(res);
            }
        }

        results
    }

    /// Broadcasts a signed raw transaction in parallel across all healthy endpoints simultaneously.
    /// Returns individual outcome details for all raced nodes.
    pub async fn race_broadcast_raw_tx(&self, raw_tx_hex: &str) -> Vec<BroadcastOutcome> {
        let raw_hex = if raw_tx_hex.starts_with("0x") {
            raw_tx_hex.to_string()
        } else {
            format!("0x{raw_tx_hex}")
        };

        let mut handles = Vec::new();

        for ep in &self.endpoints {
            if !ep.is_healthy {
                continue;
            }
            let client = self.client.clone();
            let url = ep.url.clone();
            let tx_data = raw_hex.clone();

            handles.push(tokio::spawn(async move {
                let payload = json!({
                    "jsonrpc": "2.0",
                    "method": "eth_sendRawTransaction",
                    "params": [tx_data],
                    "id": 1
                });

                let start = Instant::now();
                let outcome = match client.post(&url).json(&payload).send().await {
                    Ok(resp) => {
                        let duration_ms = start.elapsed().as_millis() as u64;
                        if let Ok(json_body) = resp.json::<Value>().await {
                            if let Some(tx_hash) = json_body.get("result").and_then(|r| r.as_str()) {
                                BroadcastOutcome {
                                    rpc_url: url,
                                    duration_ms,
                                    success: true,
                                    tx_hash: Some(tx_hash.to_string()),
                                    error: None,
                                }
                            } else if let Some(err) = json_body.get("error") {
                                BroadcastOutcome {
                                    rpc_url: url,
                                    duration_ms,
                                    success: false,
                                    tx_hash: None,
                                    error: Some(err.to_string()),
                                }
                            } else {
                                BroadcastOutcome {
                                    rpc_url: url,
                                    duration_ms,
                                    success: false,
                                    tx_hash: None,
                                    error: Some("Unknown JSON-RPC response format".to_string()),
                                }
                            }
                        } else {
                            BroadcastOutcome {
                                rpc_url: url,
                                duration_ms,
                                success: false,
                                tx_hash: None,
                                error: Some("Failed to decode JSON response".to_string()),
                            }
                        }
                    }
                    Err(e) => BroadcastOutcome {
                        rpc_url: url,
                        duration_ms: start.elapsed().as_millis() as u64,
                        success: false,
                        tx_hash: None,
                        error: Some(e.to_string()),
                    },
                };
                outcome
            }));
        }

        let mut outcomes = Vec::new();
        for handle in handles {
            if let Ok(res) = handle.await {
                outcomes.push(res);
            }
        }

        outcomes
    }

    /// Fetches transaction receipt concurrently from all healthy endpoints, returning the fastest valid receipt.
    pub async fn poll_receipt(&self, tx_hash: &str) -> Option<Value> {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": "eth_getTransactionReceipt",
            "params": [tx_hash],
            "id": 1
        });

        let mut tasks = Vec::new();
        for ep in &self.endpoints {
            if !ep.is_healthy {
                continue;
            }
            let client = self.client.clone();
            let url = ep.url.clone();
            let p = payload.clone();
            tasks.push(tokio::spawn(async move {
                if let Ok(resp) = client.post(&url).json(&p).send().await {
                    if let Ok(json_body) = resp.json::<Value>().await {
                        if let Some(receipt) = json_body.get("result") {
                            if !receipt.is_null() {
                                return Some(receipt.clone());
                            }
                        }
                    }
                }
                None
            }));
        }

        for task in tasks {
            if let Ok(Some(receipt)) = task.await {
                return Some(receipt);
            }
        }

        None
    }

    pub fn endpoints(&self) -> &[RpcEndpoint] {
        &self.endpoints
    }
}

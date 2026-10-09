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
            .map(|url| {
                let is_ws = url.starts_with("ws://") || url.starts_with("wss://");
                if is_ws {
                    eprintln!("⚠️ WARNING: WebSocket URL detected: {}. RpcRacer requires HTTP/HTTPS endpoints. Marking inactive.", url);
                }
                RpcEndpoint {
                    url: url.clone(),
                    is_healthy: !is_ws,
                }
            })
            .collect();

        Self {
            client,
            endpoints,
        }
    }

    /// Pings all configured RPC endpoints concurrently to evaluate response times and keep connection pools warm.
    pub async fn benchmark_latencies(&self) -> Vec<(String, Option<u64>)> {
        let mut tasks = Vec::new();

        for endpoint in &self.endpoints {
            if !endpoint.is_healthy {
                continue;
            }
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

        use futures_util::stream::{FuturesUnordered, StreamExt};
        let tasks = FuturesUnordered::new();
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

        let mut stream = tasks;
        while let Some(res) = stream.next().await {
            if let Ok(Some(receipt)) = res {
                return Some(receipt);
            }
        }

        None
    }

    /// Queries the latest block's baseFeePerGas from healthy endpoints.
    pub async fn fetch_latest_base_fee(&self) -> Option<u128> {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": "eth_getBlockByNumber",
            "params": ["latest", false],
            "id": 1
        });

        for ep in &self.endpoints {
            if !ep.is_healthy {
                continue;
            }
            if let Ok(resp) = self.client.post(&ep.url).json(&payload).send().await {
                if let Ok(json_body) = resp.json::<Value>().await {
                    if let Some(base_fee_hex) = json_body.get("result")
                        .and_then(|r| r.get("baseFeePerGas"))
                        .and_then(|b| b.as_str()) {
                        if let Ok(base_fee) = u128::from_str_radix(base_fee_hex.trim_start_matches("0x"), 16) {
                            return Some(base_fee);
                        }
                    }
                }
            }
        }
        None
    }

    pub fn endpoints(&self) -> &[RpcEndpoint] {
        &self.endpoints
    }
}

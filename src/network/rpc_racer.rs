use reqwest::Client;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// One RPC endpoint plus its live health flag.
///
/// Health used to be frozen at construction time, so a dead endpoint stayed in every
/// race forever (and a recovered one was never re-admitted). It is now updated from real
/// probe results: `benchmark_latencies` is the health probe and re-admits endpoints,
/// while broadcast/query failures demote them.
pub struct RpcEndpoint {
    pub url: String,
    healthy: AtomicBool,
}

impl RpcEndpoint {
    pub fn new(url: String, healthy: bool) -> Self {
        Self {
            url,
            healthy: AtomicBool::new(healthy),
        }
    }

    pub fn is_healthy(&self) -> bool {
        self.healthy.load(Ordering::Relaxed)
    }

    pub fn set_healthy(&self, healthy: bool) {
        self.healthy.store(healthy, Ordering::Relaxed);
    }
}

impl Clone for RpcEndpoint {
    fn clone(&self) -> Self {
        Self::new(self.url.clone(), self.is_healthy())
    }
}

impl std::fmt::Debug for RpcEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RpcEndpoint")
            .field("url", &self.url)
            .field("healthy", &self.is_healthy())
            .finish()
    }
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
                RpcEndpoint::new(url.clone(), !is_ws)
            })
            .collect();

        Self {
            client,
            endpoints,
        }
    }

    /// Pings all configured RPC endpoints concurrently to evaluate response times and keep connection pools warm.
    ///
    /// This doubles as the health probe: endpoints that fail are demoted, and previously
    /// demoted endpoints are re-probed so a transient outage cannot disable one permanently.
    pub async fn benchmark_latencies(&self) -> Vec<(String, Option<u64>)> {
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
            if let Ok((url, latency)) = task.await {
                self.mark_health(&url, latency.is_some());
                results.push((url, latency));
            }
        }

        results
    }

    /// Updates an endpoint's health flag by URL. Unknown URLs are ignored.
    fn mark_health(&self, url: &str, healthy: bool) {
        for ep in &self.endpoints {
            if ep.url == url {
                ep.set_healthy(healthy);
                return;
            }
        }
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
            if !ep.is_healthy() {
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

        // Demote endpoints that could not even complete the transport-level send.
        for outcome in &outcomes {
            if !outcome.success {
                if let Some(ref err) = outcome.error {
                    if is_transport_failure(err) {
                        self.mark_health(&outcome.rpc_url, false);
                    }
                }
            }
        }

        outcomes
    }

    /// Fetches a receipt for any of the given transaction hashes from all healthy endpoints.
    /// Returns the first non-null receipt and aborts the remaining in-flight requests.
    pub async fn poll_receipts(&self, tx_hashes: &[String]) -> Option<Value> {
        use futures::stream::{FuturesUnordered, StreamExt};

        let mut handles = Vec::new();
        let mut tasks = FuturesUnordered::new();
        for hash in tx_hashes {
            let payload = json!({
                "jsonrpc": "2.0",
                "method": "eth_getTransactionReceipt",
                "params": [hash],
                "id": 1
            });
            for ep in &self.endpoints {
                if !ep.is_healthy() {
                    continue;
                }
                let client = self.client.clone();
                let url = ep.url.clone();
                let p = payload.clone();
                let handle = tokio::spawn(async move {
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
                });
                handles.push(handle.abort_handle());
                tasks.push(handle);
            }
        }

        let mut found = None;
        while let Some(res) = tasks.next().await {
            if let Ok(Some(receipt)) = res {
                found = Some(receipt);
                break;
            }
        }

        for handle in handles {
            handle.abort();
        }
        found
    }

    /// Fetches a receipt for a single transaction hash.
    pub async fn poll_receipt(&self, tx_hash: &str) -> Option<Value> {
        self.poll_receipts(&[tx_hash.to_string()]).await
    }

    /// Races an arbitrary JSON-RPC query across all healthy endpoints in parallel,
    /// returning the first valid parsed result and aborting remaining in-flight requests.
    async fn race_query<T: Send + 'static>(
        &self,
        payload: Value,
        parser: fn(&Value) -> Option<T>,
    ) -> Option<T> {
        use futures::stream::{FuturesUnordered, StreamExt};
        let mut handles = Vec::new();
        let mut tasks = FuturesUnordered::new();

        for ep in &self.endpoints {
            if !ep.is_healthy() {
                continue;
            }
            let client = self.client.clone();
            let url = ep.url.clone();
            let p = payload.clone();
            let handle = tokio::spawn(async move {
                if let Ok(resp) = client.post(&url).json(&p).send().await {
                    if let Ok(json_body) = resp.json::<Value>().await {
                        if let Some(res) = json_body.get("result") {
                            return parser(res);
                        }
                    }
                }
                None
            });
            handles.push(handle.abort_handle());
            tasks.push(handle);
        }

        let mut found = None;
        while let Some(res) = tasks.next().await {
            if let Ok(Some(val)) = res {
                found = Some(val);
                break;
            }
        }
        for h in handles {
            h.abort();
        }
        found
    }

    /// Queries the latest block's baseFeePerGas across endpoints in parallel.
    pub async fn fetch_latest_base_fee(&self) -> Option<u128> {
        let payload = json!({"jsonrpc": "2.0", "method": "eth_getBlockByNumber", "params": ["latest", false], "id": 1});
        self.race_query(payload, |r| {
            let hex_str = r.get("baseFeePerGas")?.as_str()?;
            u128::from_str_radix(hex_str.trim_start_matches("0x"), 16).ok()
        }).await
    }

    /// Queries the latest block's timestamp across endpoints in parallel.
    pub async fn fetch_latest_block_timestamp(&self) -> Option<u64> {
        let payload = json!({"jsonrpc": "2.0", "method": "eth_getBlockByNumber", "params": ["latest", false], "id": 1});
        self.race_query(payload, |r| {
            let hex_str = r.get("timestamp")?.as_str()?;
            u64::from_str_radix(hex_str.trim_start_matches("0x"), 16).ok()
        }).await
    }

    /// Queries the latest confirmed block number across endpoints in parallel.
    pub async fn fetch_latest_block_number(&self) -> Option<u64> {
        let payload = json!({"jsonrpc": "2.0", "method": "eth_blockNumber", "params": [], "id": 1});
        self.race_query(payload, |r| {
            let hex_str = r.as_str()?;
            u64::from_str_radix(hex_str.trim_start_matches("0x"), 16).ok()
        }).await
    }

    /// Queries the on-chain confirmed transaction count (nonce) for an address in parallel.
    pub async fn fetch_transaction_count(&self, address: &str) -> Option<u64> {
        let payload = json!({"jsonrpc": "2.0", "method": "eth_getTransactionCount", "params": [address, "latest"], "id": 1});
        self.race_query(payload, |r| {
            let hex_str = r.as_str()?;
            u64::from_str_radix(hex_str.trim_start_matches("0x"), 16).ok()
        }).await
    }

    /// Queries the on-chain balance in Wei for an address across endpoints in parallel.
    pub async fn fetch_balance(&self, address: &str) -> Option<u128> {
        let payload = json!({"jsonrpc": "2.0", "method": "eth_getBalance", "params": [address, "latest"], "id": 1});
        self.race_query(payload, |r| {
            let hex_str = r.as_str()?;
            u128::from_str_radix(hex_str.trim_start_matches("0x"), 16).ok()
        }).await
    }

    pub fn endpoints(&self) -> &[RpcEndpoint] {
        &self.endpoints
    }
}

/// Distinguishes "this endpoint is broken" from "this endpoint answered and refused".
///
/// A node that rejects a transaction on its merits (`nonce too low`, `already known`,
/// `insufficient funds`, `replacement transaction underpriced`) is healthy — demoting it
/// would disable a perfectly good node during normal operation. Only transport-level and
/// rate-limit failures indicate an endpoint that should be taken out of the rotation.
fn is_transport_failure(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    const NODE_VERDICTS: [&str; 8] = [
        "nonce too low",
        "already known",
        "known transaction",
        "replacement transaction underpriced",
        "insufficient funds",
        "intrinsic gas too low",
        "exceeds block gas limit",
        "execution reverted",
    ];
    if NODE_VERDICTS.iter().any(|verdict| lower.contains(verdict)) {
        return false;
    }

    // reqwest transport errors, HTTP statuses surfaced by error_for_status, and rate limits.
    const TRANSPORT_MARKERS: [&str; 7] = [
        "error sending request",
        "connection refused",
        "connection reset",
        "timed out",
        "timeout",
        "429",
        "503",
    ];
    TRANSPORT_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_verdicts_do_not_demote_endpoints() {
        for verdict in [
            "nonce too low",
            "ALREADY KNOWN",
            "replacement transaction underpriced",
            "insufficient funds for gas * price + value",
        ] {
            assert!(
                !is_transport_failure(verdict),
                "{verdict:?} is a node verdict, not a transport failure"
            );
        }
    }

    #[test]
    fn transport_failures_demote_endpoints() {
        for failure in [
            "error sending request for url (https://rpc.example)",
            "connection refused",
            "operation timed out",
            "HTTP 429 Too Many Requests",
        ] {
            assert!(
                is_transport_failure(failure),
                "{failure:?} should demote the endpoint"
            );
        }
    }

    #[test]
    fn health_flag_round_trips() {
        let ep = RpcEndpoint::new("https://a.example".to_string(), true);
        assert!(ep.is_healthy());
        ep.set_healthy(false);
        assert!(!ep.is_healthy());
        // Clone must carry the current flag, not a stale snapshot.
        let cloned = ep.clone();
        assert!(!cloned.is_healthy());
    }

    #[test]
    fn websocket_endpoints_are_marked_inactive() {
        let racer = RpcRacer::new(
            &["wss://ws.example".to_string(), "https://http.example".to_string()],
            1000,
        );
        let eps = racer.endpoints();
        assert!(!eps[0].is_healthy(), "ws endpoint must be inactive");
        assert!(eps[1].is_healthy(), "http endpoint must be active");
    }
}

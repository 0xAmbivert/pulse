use reqwest::Client;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::Signer;

#[derive(Debug, Clone)]
pub struct BuilderOutcome {
    pub builder_url: String,
    pub duration_ms: u64,
    pub success: bool,
    pub response: Option<String>,
    pub error: Option<String>,
}

pub struct MevBuilderClient {
    client: Client,
    builder_urls: Vec<String>,
    mev_identity: PrivateKeySigner,
}

impl MevBuilderClient {
    pub fn new(builder_urls: &[String], timeout_ms: u64) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .pool_max_idle_per_host(16)
            .tcp_nodelay(true)
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            builder_urls: builder_urls.to_vec(),
            mev_identity: PrivateKeySigner::random(),
        }
    }

    /// Dispatches a signed private transaction to direct block builders (e.g. Flashbots, Titan, BeaverBuild)
    /// to bypass the public mempool, mitigating frontrunning, sandwiching, and revert penalties.
    pub async fn send_private_transaction(&self, raw_tx_hex: &str) -> Vec<BuilderOutcome> {
        let raw_hex = if raw_tx_hex.starts_with("0x") {
            raw_tx_hex.to_string()
        } else {
            format!("0x{raw_tx_hex}")
        };

        let mut handles = Vec::new();

        for url in &self.builder_urls {
            let client = self.client.clone();
            let builder_url = url.clone();
            let tx_hex = raw_hex.clone();
            let mev_identity = self.mev_identity.clone();

            handles.push(tokio::spawn(async move {
                let start = Instant::now();
                // Standard Flashbots / Builder private transaction format: eth_sendPrivateTransaction
                let payload = json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "eth_sendPrivateTransaction",
                    "params": [{
                        "tx": tx_hex,
                        "preferences": {
                            "fast": true
                        }
                    }]
                });
                
                let payload_str = serde_json::to_string(&payload).unwrap_or_default();
                let sig_result = mev_identity.sign_message(alloy::primitives::keccak256(payload_str.as_bytes()).as_slice()).await;
                if let Ok(sig) = sig_result {
                    let auth_header = format!("{:?}:0x{}", mev_identity.address(), hex::encode(sig.as_bytes()));

                    match client.post(&builder_url).header("X-Flashbots-Signature", auth_header).json(&payload).send().await {
                        Ok(resp) => {
                            let duration_ms = start.elapsed().as_millis() as u64;
                            if let Ok(body) = resp.json::<Value>().await {
                                if let Some(err) = body.get("error") {
                                    BuilderOutcome {
                                        builder_url,
                                        duration_ms,
                                        success: false,
                                        response: None,
                                        error: Some(err.to_string()),
                                    }
                                } else {
                                    BuilderOutcome {
                                        builder_url,
                                        duration_ms,
                                        success: true,
                                        response: body.get("result").map(|r| r.to_string()),
                                        error: None,
                                    }
                                }
                            } else {
                                BuilderOutcome {
                                    builder_url,
                                    duration_ms,
                                    success: false,
                                    response: None,
                                    error: Some("Failed to decode response".to_string()),
                                }
                            }
                        }
                        Err(e) => BuilderOutcome {
                            builder_url,
                            duration_ms: start.elapsed().as_millis() as u64,
                            success: false,
                            response: None,
                            error: Some(e.to_string()),
                        },
                    }
                } else {
                    BuilderOutcome {
                        builder_url,
                        duration_ms: start.elapsed().as_millis() as u64,
                        success: false,
                        response: None,
                        error: Some("Failed to sign payload for builder".to_string()),
                    }
                }
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
}

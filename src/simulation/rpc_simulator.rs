

use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct SimResult {
    pub success: bool,
    pub gas_used: u64,
    pub revert_reason: Option<String>,
}

pub struct RpcSimulator {
    rpc_url: String,
    client: Client,
}

impl RpcSimulator {
    pub fn new(rpc_url: &str) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_millis(5000))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            rpc_url: rpc_url.to_string(),
            client,
        }
    }

    /// Simulates transaction execution via remote RPC `eth_estimateGas` to detect honeypots/reverts.
    pub async fn simulate_call(
        &self,
        caller: &str,
        target_contract: &str,
        calldata: &[u8],
        value_wei: u128,
        _gas_limit: u64,
    ) -> Result<SimResult, Box<dyn std::error::Error + Send + Sync>> {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": "eth_estimateGas",
            "params": [{
                "from": caller,
                "to": target_contract,
                "data": format!("0x{}", hex::encode(calldata)),
                "value": format!("0x{:x}", value_wei)
            }, "latest"],
            "id": 1
        });

        let resp = self.client.post(&self.rpc_url).json(&payload).send().await?;
        let body: Value = resp.json().await?;

        if let Some(err) = body.get("error") {
            let revert_reason = err.get("message").and_then(|m| m.as_str()).unwrap_or("Unknown revert");
            return Ok(SimResult {
                success: false,
                gas_used: 0,
                revert_reason: Some(revert_reason.to_string()),
            });
        }

        let gas_used = if let Some(res) = body.get("result").and_then(|r| r.as_str()) {
            u64::from_str_radix(res.trim_start_matches("0x"), 16).unwrap_or(21000)
        } else {
            21000
        };

        Ok(SimResult {
            success: true,
            gas_used,
            revert_reason: None,
        })
    }
}



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
    /// Distinguishes "the call reverts" from "we could not find out".
    ///
    /// Returns `Ok(Some(gas))` on success, `Ok(None)` when the node answered and the call
    /// reverted, and `Err` when the simulation itself failed. Callers must not treat `Err`
    /// as a revert: a 429 or a timeout from a public RPC says nothing about the contract.
    pub async fn simulate_call(
        &self,
        caller: &str,
        target_contract: &str,
        calldata: &[u8],
        value_wei: u128,
        gas_limit: u64,
    ) -> Result<Option<SimResult>, Box<dyn std::error::Error + Send + Sync>> {
        let mut tx_obj = json!({
            "from": caller,
            "to": target_contract,
            "data": format!("0x{}", hex::encode(calldata)),
            "value": format!("0x{:x}", value_wei),
        });
        if gas_limit > 0 {
            tx_obj["gas"] = json!(format!("0x{:x}", gas_limit));
        }
        let payload = json!({
            "jsonrpc": "2.0",
            "method": "eth_estimateGas",
            "params": [tx_obj, "latest"],
            "id": 1
        });

        let resp = self.client.post(&self.rpc_url).json(&payload).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!(
                "simulation endpoint returned HTTP {status} (not a contract revert)"
            )
            .into());
        }
        let body: Value = resp.json().await?;

        if let Some(err) = body.get("error") {
            let revert_reason = err.get("message").and_then(|m| m.as_str()).unwrap_or("Unknown revert");
            // A JSON-RPC transport-level failure (rate limit, node overload) carries an
            // error object but is not an execution result.
            if let Some(code) = err.get("code").and_then(|c| c.as_i64()) {
                let node_level = matches!(code, -32005 | -32603 | -32000);
                if node_level && err.get("data").is_none() {
                    return Err(format!("simulation failed at the node (code {code}): {revert_reason}").into());
                }
            }
            return Ok(Some(SimResult {
                success: false,
                gas_used: 0,
                revert_reason: Some(revert_reason.to_string()),
            }));
        }

        let res = body
            .get("result")
            .and_then(|r| r.as_str())
            .ok_or("eth_estimateGas returned no result")?;
        let gas_used = u64::from_str_radix(res.trim_start_matches("0x"), 16)?;

        Ok(Some(SimResult {
            success: true,
            gas_used,
            revert_reason: None,
        }))
    }
}

use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;
use crate::eligibility::types::SeaDropPublicConfig;

pub const SEADROP_V1_ADDRESS: &str = "0x00005ea00ac477b1030ce78506496e8c2de24bf5";

pub struct SeaDropInspector {
    rpc_url: String,
    client: Client,
}

impl SeaDropInspector {
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

    /// Queries on-chain getPublicDrop(address) on the SeaDrop contract.
    pub async fn fetch_public_drop(&self, nft_contract: &str) -> Option<SeaDropPublicConfig> {
        // Selector 0xbc6a629c
        let clean = nft_contract.trim().trim_start_matches("0x").to_lowercase();
        let calldata = format!("0xbc6a629c{:0>64}", clean);

        let res = self.eth_call(SEADROP_V1_ADDRESS, &calldata, "0x0").await.ok()?;
        let hex_res = res.trim_start_matches("0x");

        if hex_res.len() < 384 {
            return None;
        }

        let mint_price_wei = u128::from_str_radix(&hex_res[0..64], 16).ok()?;
        let start_time = u64::from_str_radix(&hex_res[64..128], 16).ok()?;
        let end_time = u64::from_str_radix(&hex_res[128..192], 16).ok()?;
        let max_per_wallet = u64::from_str_radix(&hex_res[192..256], 16).ok()?;
        let fee_bps = u16::from_str_radix(&hex_res[256..320], 16).ok()?;

        let fee_recipient = self.fetch_allowed_fee_recipient(nft_contract).await;

        Some(SeaDropPublicConfig {
            mint_price_wei,
            start_time,
            end_time,
            max_per_wallet,
            fee_bps,
            fee_recipient,
        })
    }

    /// Queries getAllowedFeeRecipients(address) on SeaDrop.
    pub async fn fetch_allowed_fee_recipient(&self, nft_contract: &str) -> Option<String> {
        let clean = nft_contract.trim().trim_start_matches("0x").to_lowercase();
        let calldata = format!("0x68632274{:0>64}", clean);
        let res = self.eth_call(SEADROP_V1_ADDRESS, &calldata, "0x0").await.ok()?;
        let hex_res = res.trim_start_matches("0x");
        if hex_res.len() >= 192 {
            let addr_hex = &hex_res[hex_res.len() - 40..];
            return Some(format!("0x{addr_hex}"));
        }
        None
    }

    /// Queries getMintStats(address minter) on the NFT contract.
    pub async fn fetch_nft_mint_stats(&self, nft_contract: &str, minter: &str) -> Option<(u64, u64, u64)> {
        let clean = minter.trim().trim_start_matches("0x").to_lowercase();
        let calldata = format!("0x840e15d4{:0>64}", clean);
        let res = self.eth_call(nft_contract, &calldata, "0x0").await.ok()?;
        let hex_res = res.trim_start_matches("0x");
        if hex_res.len() >= 192 {
            let minted = u64::from_str_radix(&hex_res[0..64], 16).ok()?;
            let current = u64::from_str_radix(&hex_res[64..128], 16).ok()?;
            let max = u64::from_str_radix(&hex_res[128..192], 16).ok()?;
            return Some((minted, current, max));
        }
        None
    }

    /// Queries totalSupply() and maxSupply() on an arbitrary ERC-721 contract.
    pub async fn fetch_supplies(&self, nft_contract: &str) -> (Option<u64>, Option<u64>) {
        let total = self.eth_call(nft_contract, "0x18160ddd", "0x0").await.ok()
            .and_then(|r| u64::from_str_radix(r.trim_start_matches("0x"), 16).ok());
        let max = self.eth_call(nft_contract, "0xd5abeb01", "0x0").await.ok()
            .and_then(|r| u64::from_str_radix(r.trim_start_matches("0x"), 16).ok());
        (total, max)
    }

    /// Simulates a mint call from the user's wallet via eth_call and eth_estimateGas.
    pub async fn simulate_mint(
        &self,
        from_wallet: &str,
        to_target: &str,
        calldata: &str,
        value_hex: &str,
    ) -> Result<u64, String> {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": "eth_estimateGas",
            "params": [{
                "from": from_wallet,
                "to": to_target,
                "data": calldata,
                "value": value_hex
            }, "latest"],
            "id": 1
        });

        let resp = self.client.post(&self.rpc_url).json(&payload).send().await
            .map_err(|e| format!("RPC transport error: {e}"))?;
        let body: Value = resp.json().await
            .map_err(|e| format!("Failed to parse RPC response: {e}"))?;

        if let Some(err) = body.get("error") {
            let revert_data = err.get("data").and_then(|d| d.as_str()).unwrap_or("");
            let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("Execution reverted");
            let decoded = decode_revert_reason(revert_data, msg);
            return Err(decoded);
        }

        if let Some(res) = body.get("result").and_then(|r| r.as_str()) {
            let gas = u64::from_str_radix(res.trim_start_matches("0x"), 16).unwrap_or(120_000);
            return Ok(gas);
        }

        Err("Unknown RPC response".to_string())
    }

    async fn eth_call(&self, to: &str, data: &str, value: &str) -> Result<String, String> {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": "eth_call",
            "params": [{
                "to": to,
                "data": data,
                "value": value
            }, "latest"],
            "id": 1
        });

        let resp = self.client.post(&self.rpc_url).json(&payload).send().await
            .map_err(|e| format!("RPC error: {e}"))?;
        let body: Value = resp.json().await
            .map_err(|e| format!("RPC JSON error: {e}"))?;

        if let Some(err) = body.get("error") {
            let revert_data = err.get("data").and_then(|d| d.as_str()).unwrap_or("");
            let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("Reverted");
            return Err(decode_revert_reason(revert_data, msg));
        }

        body.get("result").and_then(|r| r.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "Empty eth_call result".to_string())
    }
}

/// Decodes common SeaDrop and ERC-721 custom error selectors into human-readable reasons.
pub fn decode_revert_reason(data: &str, fallback_msg: &str) -> String {
    let clean = data.trim_start_matches("0x");
    if clean.len() >= 8 {
        let selector = &clean[0..8];
        match selector {
            "e12d2314" => return "Drop is Sold Out (MintQuantityExceedsMaxSupply)".to_string(),
            "a6fd7170" => return "Drop Stage is Not Active (NotActive)".to_string(),
            "d845e2c5" => return "Wallet Limit Reached (MintQuantityExceedsMaxMintedPerWallet)".to_string(),
            "09bde339" => return "Not on Allowlist / Invalid Merkle Proof (InvalidProof)".to_string(),
            "d368bb8d" => return "Incorrect Payment Value Sent (IncorrectPayment)".to_string(),
            "c8bf2286" => return "Mint Quantity Cannot Be Zero (MintQuantityCannotBeZero)".to_string(),
            "9484b80b" => return "Signer Not Present on Drop (SignerNotPresent)".to_string(),
            "8baa579f" => return "Invalid Server-Side Signature (InvalidSignature)".to_string(),
            "5e548816" => return "Fee Recipient Not Allowed (FeeRecipientNotAllowed)".to_string(),
            "4e487b71" => return "Panic / Integer Overflow (Panic)".to_string(),
            "08c379a0" if clean.len() >= 136 => {
                // Standard Error(string)
                if let Ok(bytes) = hex::decode(&clean[136..]) {
                    if let Ok(s) = String::from_utf8(bytes) {
                        return s.trim_matches(char::from(0)).to_string();
                    }
                }
            }
            _ => {}
        }
    }
    fallback_msg.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_seadrop_revert_reasons() {
        assert_eq!(
            decode_revert_reason("0xe12d2314000000", "revert"),
            "Drop is Sold Out (MintQuantityExceedsMaxSupply)"
        );
        assert_eq!(
            decode_revert_reason("0xa6fd7170000000", "revert"),
            "Drop Stage is Not Active (NotActive)"
        );
        assert_eq!(
            decode_revert_reason("0x09bde339000000", "revert"),
            "Not on Allowlist / Invalid Merkle Proof (InvalidProof)"
        );
    }
}

use reqwest::Client;
use serde_json::Value;
use std::time::Duration;
use crate::eligibility::types::ResolvedDropTarget;

pub struct OpenSeaClient {
    client: Client,
}

impl Default for OpenSeaClient {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenSeaClient {
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_millis(6000))
            .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .build()
            .unwrap_or_else(|_| Client::new());
        Self { client }
    }

    /// Extracts collection slug from an OpenSea URL or returns the input if already a slug.
    pub fn extract_slug(input: &str) -> Option<String> {
        let trimmed = input.trim();
        if trimmed.contains("opensea.io/collection/") {
            let part = trimmed.split("opensea.io/collection/").nth(1)?;
            let slug = part.split('/').next()?.split('?').next()?;
            if !slug.is_empty() {
                return Some(slug.to_lowercase());
            }
        } else if !trimmed.starts_with("0x") && !trimmed.contains("://") && !trimmed.is_empty() {
            return Some(trimmed.to_lowercase());
        }
        None
    }

    /// Builds a GET request, attaching `X-API-KEY` from `OPENSEA_API_KEY` when set.
    fn get(&self, url: &str) -> reqwest::RequestBuilder {
        let req = self.client.get(url).header("Accept", "application/json");
        match std::env::var("OPENSEA_API_KEY") {
            Ok(key) if !key.trim().is_empty() => req.header("X-API-KEY", key.trim()),
            _ => req,
        }
    }

    /// Resolves an OpenSea collection by slug using OpenSea API v2.
    pub async fn fetch_collection_by_slug(&self, slug: &str) -> Result<ResolvedDropTarget, String> {
        let url = format!("https://api.opensea.io/api/v2/collections/{slug}");
        let resp = self.get(&url)
            .send().await
            .map_err(|e| format!("OpenSea API connection error: {e}"))?;

        if resp.status().as_u16() == 401 {
            return Err("OpenSea returned HTTP 401. Set OPENSEA_API_KEY to a valid key.".to_string());
        }
        if !resp.status().is_success() {
            return Err(format!("OpenSea collection '{}' not found (HTTP {})", slug, resp.status()));
        }

        let body: Value = resp.json().await
            .map_err(|e| format!("Failed to parse OpenSea response: {e}"))?;

        let name = body.get("name")
            .and_then(|n| n.as_str())
            .unwrap_or(slug)
            .to_string();

        let contracts = body.get("contracts")
            .and_then(|c| c.as_array())
            .ok_or_else(|| "No contracts found for collection".to_string())?;

        let first_contract = contracts.first()
            .ok_or_else(|| "Collection has empty contracts list".to_string())?;

        let address = first_contract.get("address")
            .and_then(|a| a.as_str())
            .ok_or_else(|| "Contract missing address".to_string())?
            .to_lowercase();

        let chain_str = first_contract.get("chain")
            .and_then(|c| c.as_str())
            .unwrap_or("ethereum");

        let chain_id = chain_slug_to_id(chain_str)?;

        let fee_recipient = body.get("fees")
            .and_then(|f| f.as_array())
            .and_then(|arr| arr.iter().find(|fee| fee.get("required").and_then(|r| r.as_bool()).unwrap_or(false)))
            .and_then(|f| f.get("recipient"))
            .and_then(|r| r.as_str())
            .map(|s| s.to_string());

        Ok(ResolvedDropTarget {
            contract_address: address,
            collection_name: name,
            collection_slug: Some(slug.to_string()),
            chain: chain_str.to_string(),
            chain_id,
            opensea_url: Some(format!("https://opensea.io/collection/{slug}")),
            fee_recipient,
        })
    }

    /// Resolves an OpenSea collection by contract address and chain.
    pub async fn fetch_collection_by_contract(&self, chain: &str, contract: &str) -> Result<ResolvedDropTarget, String> {
        let url = format!("https://api.opensea.io/api/v2/chain/{chain}/contract/{contract}");
        let resp = self.get(&url)
            .send().await
            .map_err(|e| format!("OpenSea API connection error: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("OpenSea contract lookup returned HTTP {}", resp.status()));
        }

        let body: Value = resp.json().await
            .map_err(|e| format!("Failed to parse OpenSea response: {e}"))?;

        let slug = body.get("collection")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string());

        let name = body.get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("Unknown NFT")
            .to_string();

        let opensea_url = slug.as_ref().map(|s| format!("https://opensea.io/collection/{s}"));

        // If slug exists, fetch full collection for fee recipient
        let fee_recipient = if let Some(ref s) = slug {
            self.fetch_collection_by_slug(s).await.ok().and_then(|t| t.fee_recipient)
        } else {
            None
        };

        Ok(ResolvedDropTarget {
            contract_address: contract.to_lowercase(),
            collection_name: name,
            collection_slug: slug,
            chain: chain.to_string(),
            chain_id: chain_slug_to_id(chain)?,
            opensea_url,
            fee_recipient,
        })
    }
}

/// Maps an OpenSea chain slug to a chain ID. Unknown slugs are an error, never a silent default.
pub fn chain_slug_to_id(chain: &str) -> Result<u64, String> {
    match chain.to_lowercase().as_str() {
        "robinhood" => Ok(4663),
        "ethereum" | "eth" | "mainnet" => Ok(1),
        "base" => Ok(8453),
        "arbitrum" | "arbitrum_one" => Ok(42161),
        "polygon" | "matic" => Ok(137),
        "optimism" => Ok(10),
        "sepolia" => Ok(11155111),
        other => Err(format!("Unsupported chain slug '{other}'")),
    }
}

/// Maps a chain ID to an OpenSea chain slug. Unknown IDs are an error.
pub fn chain_id_to_slug(chain_id: u64) -> Result<&'static str, String> {
    match chain_id {
        4663 => Ok("robinhood"),
        1 => Ok("ethereum"),
        8453 => Ok("base"),
        42161 => Ok("arbitrum"),
        137 => Ok("polygon"),
        10 => Ok("optimism"),
        11155111 => Ok("sepolia"),
        other => Err(format!("Unsupported chain ID {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_slug_from_urls() {
        assert_eq!(
            OpenSeaClient::extract_slug("https://opensea.io/collection/robinhoodmisfits"),
            Some("robinhoodmisfits".to_string())
        );
        assert_eq!(
            OpenSeaClient::extract_slug("https://opensea.io/collection/housecraft/drop?tab=details"),
            Some("housecraft".to_string())
        );
        assert_eq!(
            OpenSeaClient::extract_slug("cool-bears"),
            Some("cool-bears".to_string())
        );
        assert_eq!(
            OpenSeaClient::extract_slug("0x0c5f73b594942744323c7451c2fce3a779cb080b"),
            None
        );
    }

    #[test]
    fn test_chain_mapping() {
        assert_eq!(chain_slug_to_id("robinhood"), Ok(4663));
        assert_eq!(chain_slug_to_id("base"), Ok(8453));
        assert_eq!(chain_id_to_slug(4663), Ok("robinhood"));
        assert!(chain_slug_to_id("made-up").is_err());
        assert!(chain_id_to_slug(999_999).is_err());
    }
}

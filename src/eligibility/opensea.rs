use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;
use crate::crypto::{create_signer_from_protected, get_address_from_protected, ProtectedKey};
use crate::eligibility::types::{OpenSeaEligibilityResult, ResolvedDropTarget};

pub struct OpenSeaClient {
    client: Client,
    pub auth_token: Option<String>,
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
        Self {
            client,
            auth_token: None,
        }
    }

    pub fn with_token(token: &str) -> Self {
        let mut client = Self::new();
        client.auth_token = Some(token.to_string());
        client
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

    /// Attaches standard API key and Bearer auth headers if available.
    fn attach_headers(&self, mut req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req = req.header("Accept", "application/json");
        if let Ok(key) = std::env::var("OPENSEA_API_KEY") {
            if !key.trim().is_empty() {
                req = req.header("X-API-KEY", key.trim());
            }
        }
        if let Some(ref token) = self.auth_token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }
        req
    }

    /// Builds a GET request with attached headers.
    fn get(&self, url: &str) -> reqwest::RequestBuilder {
        self.attach_headers(self.client.get(url))
    }

    /// Builds a standard EIP-4361 SIWE message for OpenSea login.
    pub fn build_siwe_message(address: &str, nonce: &str, chain_id: u64, issued_at: &str) -> String {
        format!(
            "opensea.io wants you to sign in with your Ethereum account:\n\
            {address}\n\n\
            Welcome to OpenSea!\n\n\
            URI: https://opensea.io\n\
            Version: 1\n\
            Chain ID: {chain_id}\n\
            Nonce: {nonce}\n\
            Issued At: {issued_at}"
        )
    }

    /// Signs an arbitrary message using EIP-191 personal_sign with a ProtectedKey.
    pub fn sign_siwe_message(key: &ProtectedKey, message: &str) -> Result<String, String> {
        let signer = create_signer_from_protected(key)
            .map_err(|e| format!("Signer initialization error: {e}"))?;
        let hash = alloy::primitives::eip191_hash_message(message.as_bytes());
        use alloy::signers::SignerSync;
        let sig = signer.sign_hash_sync(&hash)
            .map_err(|e| format!("Failed to sign message: {e}"))?;
        Ok(format!("0x{}", hex::encode(sig.as_bytes())))
    }

    /// Fetches an authentication nonce from OpenSea for a given wallet address.
    pub async fn fetch_siwe_nonce(&self, address: &str) -> Result<String, String> {
        let url = format!("https://api.opensea.io/api/v2/auth/nonce?address={address}");
        let resp = self.get(&url)
            .send().await
            .map_err(|e| format!("OpenSea nonce request transport error: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("OpenSea nonce request returned HTTP {}", resp.status()));
        }

        let body: Value = resp.json().await
            .map_err(|e| format!("Failed to parse OpenSea nonce response: {e}"))?;

        body.get("nonce")
            .and_then(|n| n.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "Missing 'nonce' field in OpenSea response".to_string())
    }

    /// Completes SIWE login by sending the signed challenge and storing the JWT token.
    pub async fn login_with_signature(&mut self, message: &str, signature: &str) -> Result<String, String> {
        let url = "https://api.opensea.io/api/v2/auth/login";
        let payload = json!({
            "message": message,
            "signature": signature,
        });

        let resp = self.attach_headers(self.client.post(url).json(&payload))
            .send().await
            .map_err(|e| format!("OpenSea login request transport error: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("OpenSea login failed with HTTP {}", resp.status()));
        }

        let body: Value = resp.json().await
            .map_err(|e| format!("Failed to parse OpenSea login response: {e}"))?;

        let token = body.get("token")
            .or_else(|| body.get("jwt"))
            .or_else(|| body.get("access_token"))
            .and_then(|t| t.as_str())
            .ok_or_else(|| "No token found in OpenSea login response".to_string())?
            .to_string();

        self.auth_token = Some(token.clone());
        Ok(token)
    }

    /// End-to-end self-sign-in flow: requests nonce, constructs EIP-4361 message,
    /// signs locally in RAM with `ProtectedKey`, and exchanges for OpenSea session JWT.
    pub async fn authenticate_with_key(&mut self, key: &ProtectedKey, chain_id: u64) -> Result<String, String> {
        let address = get_address_from_protected(key)
            .map_err(|e| format!("Failed to derive address from key: {e}"))?;

        let nonce = self.fetch_siwe_nonce(&address).await?;
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let message = Self::build_siwe_message(&address, &nonce, chain_id, &now);
        let signature = Self::sign_siwe_message(key, &message)?;

        self.login_with_signature(&message, &signature).await
    }

    /// Queries wallet drop eligibility and Merkle proof using the current authenticated session.
    pub async fn fetch_drop_eligibility(&self, slug: &str, wallet_address: &str) -> Result<OpenSeaEligibilityResult, String> {
        let url = format!("https://api.opensea.io/api/v2/drop/{slug}/eligibility?address={wallet_address}");
        let resp = self.get(&url)
            .send().await
            .map_err(|e| format!("OpenSea drop eligibility transport error: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("OpenSea eligibility query returned HTTP {}", resp.status()));
        }

        let body: Value = resp.json().await
            .map_err(|e| format!("Failed to parse eligibility response: {e}"))?;

        let is_eligible = body.get("eligible")
            .or_else(|| body.get("is_eligible"))
            .and_then(|b| b.as_bool())
            .unwrap_or(true);

        let max_mintable = body.get("max_mintable")
            .or_else(|| body.get("allowed_quantity"))
            .or_else(|| body.get("max_per_wallet"))
            .and_then(|m| m.as_u64());

        let price_wei = body.get("price")
            .or_else(|| body.get("price_wei"))
            .and_then(|p| p.as_str())
            .and_then(|s| s.parse::<u128>().ok())
            .or_else(|| body.get("price").and_then(|p| p.as_u64()).map(|u| u as u128));

        let merkle_proof = body.get("proof")
            .or_else(|| body.get("merkle_proof"))
            .and_then(|p| p.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| {
                        let clean = s.trim().trim_start_matches("0x");
                        format!("0x{clean}")
                    })
                    .collect()
            })
            .unwrap_or_default();

        let stage_name = body.get("stage_name")
            .or_else(|| body.get("stage"))
            .and_then(|s| s.as_str())
            .map(|s| s.to_string());

        let signature = body.get("signature")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string());

        Ok(OpenSeaEligibilityResult {
            is_eligible,
            max_mintable,
            price_wei,
            merkle_proof,
            stage_name,
            signature,
        })
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

    #[test]
    fn test_siwe_message_and_signing() {
        let key = ProtectedKey::from_hex("4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f3608a9").unwrap();
        let expected_addr = get_address_from_protected(&key).unwrap();
        let message = OpenSeaClient::build_siwe_message(&expected_addr, "nonce123", 1, "2026-10-18T00:00:00Z");
        assert!(message.contains(&expected_addr));
        assert!(message.contains("Nonce: nonce123"));
        assert!(message.contains("Chain ID: 1"));

        let sig_hex = OpenSeaClient::sign_siwe_message(&key, &message).unwrap();
        assert!(sig_hex.starts_with("0x"));
        assert_eq!(sig_hex.len(), 132); // 0x + 65 bytes (130 chars)

        // Verify that the signature recovers the address
        let hash = alloy::primitives::eip191_hash_message(message.as_bytes());
        let sig_bytes = hex::decode(sig_hex.trim_start_matches("0x")).unwrap();
        let sig = alloy::primitives::Signature::try_from(sig_bytes.as_slice()).unwrap();
        let recovered = sig.recover_address_from_prehash(&hash).unwrap();
        assert_eq!(format!("{:#x}", recovered), expected_addr);
    }
}

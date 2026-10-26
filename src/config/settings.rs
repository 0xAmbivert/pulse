use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::str::FromStr;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub chain: ChainConfig,
    pub drop: DropConfig,
    pub gas: GasConfig,
    pub alerts: AlertConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainConfig {
    pub chain_id: u64,
    pub name: String,
    pub rpc_urls: Vec<String>,
    #[serde(default)]
    pub ws_rpc_url: Option<String>,
    pub mev_builder_urls: Vec<String>,
    pub rpc_timeout_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TriggerMode {
    #[default]
    StatePoller,
    Countdown,
    Mempool,
}

fn default_mint_quantity() -> u64 {
    1
}

fn default_mempool_flip_signatures() -> Vec<String> {
    vec![
        "setPublicSaleActive(bool)".to_string(),
        "setSaleActive(bool)".to_string(),
        "flipSaleState()".to_string(),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropConfig {
    pub target_contract: String,
    pub mint_function: String,
    pub mint_value_wei: String,
    #[serde(default = "default_mint_quantity")]
    pub mint_quantity: u64,
    #[serde(default)]
    pub trigger_mode: TriggerMode,
    #[serde(default)]
    pub merkle_proof: Vec<String>,
    #[serde(default)]
    pub allowlist_end_time: Option<u64>,
    #[serde(default)]
    pub allowlist_stage_index: Option<u64>,
    #[serde(default)]
    pub allowlist_max_supply: Option<u64>,
    #[serde(default)]
    pub allowlist_fee_bps: Option<u16>,
    #[serde(default)]
    pub allowlist_wallet_limit: Option<u64>,
    pub custom_calldata_hex: Option<String>,
    pub target_timestamp: Option<u64>,
    pub target_block: Option<u64>,
    pub monitor_owner_address: Option<String>,
    pub flip_function_signatures: Vec<String>,
    #[serde(default = "default_mempool_flip_signatures")]
    pub mempool_flip_signatures: Vec<String>,
    pub seadrop_contract: Option<String>,
    pub seadrop_fee_recipient: Option<String>,
}

impl Default for DropConfig {
    fn default() -> Self {
        Self {
            target_contract: "0x0000000000000000000000000000000000000000".to_string(),
            mint_function: "mint(uint256)".to_string(),
            mint_value_wei: "0".to_string(),
            mint_quantity: 1,
            trigger_mode: TriggerMode::StatePoller,
            merkle_proof: Vec::new(),
            allowlist_end_time: None,
            allowlist_stage_index: None,
            allowlist_max_supply: None,
            allowlist_fee_bps: None,
            allowlist_wallet_limit: None,
            custom_calldata_hex: None,
            target_timestamp: None,
            target_block: None,
            monitor_owner_address: None,
            flip_function_signatures: vec![
                "isPublicSaleActive()".to_string(),
                "publicSaleActive()".to_string(),
                "saleIsActive()".to_string(),
            ],
            mempool_flip_signatures: default_mempool_flip_signatures(),
            seadrop_contract: None,
            seadrop_fee_recipient: None,
        }
    }
}

type SeaDropAddrs = ([u8; 32], [u8; 32], [u8; 32]);

/// ABI-encodes a u64 as a big-endian 32-byte word.
fn u256_word(value: u64) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[24..32].copy_from_slice(&value.to_be_bytes());
    word
}

/// ABI-encodes a dynamic bytes32[] slice: 32-byte length followed by 32-byte proof hashes.
fn encode_bytes32_slice(proof: &[String]) -> Result<Vec<u8>, String> {
    let mut data = Vec::with_capacity(32 + proof.len() * 32);
    data.extend_from_slice(&u256_word(proof.len() as u64));
    for (i, p) in proof.iter().enumerate() {
        let clean = p.trim().trim_start_matches("0x");
        if clean.len() != 64 {
            return Err(format!("Proof element {i} must be 32 bytes (64 hex characters), got '{p}'"));
        }
        let bytes = hex::decode(clean)
            .map_err(|e| format!("Invalid hex in proof element {i}: {e}"))?;
        data.extend_from_slice(&bytes);
    }
    Ok(data)
}

/// ABI-encodes an address as a 32-byte word. Missing or malformed callers are errors.
fn encode_address_word(caller: Option<&str>) -> Result<[u8; 32], String> {
    let c = caller.ok_or_else(|| "Mint function requires a caller address".to_string())?;
    let addr = alloy::primitives::Address::from_str(c.trim())
        .map_err(|e| format!("Invalid caller address '{c}': {e}"))?;
    let mut word = [0u8; 32];
    word[12..32].copy_from_slice(addr.as_slice());
    Ok(word)
}

impl DropConfig {
    pub fn build_calldata(&self) -> Result<Vec<u8>, String> {
        self.build_calldata_for_caller(None, self.mint_quantity)
    }

    /// Builds calldata for `mint(...)`-style signatures. `quantity` is written to any `uint256` argument.
    pub fn build_calldata_for_caller(&self, caller: Option<&str>, quantity: u64) -> Result<Vec<u8>, String> {
        if let Some(ref hex_str) = self.custom_calldata_hex {
            let clean = hex_str.trim().trim_start_matches("0x");
            if !clean.is_empty() {
                return hex::decode(clean)
                    .map_err(|e| format!("Invalid custom calldata hex: {e}"));
            }
        }

        let func_sig = self.mint_function.trim();
        if func_sig.is_empty() {
            return Err("Mint function signature cannot be empty".to_string());
        }

        let hash = alloy::primitives::keccak256(func_sig.as_bytes());
        let mut data = hash[0..4].to_vec();

        // Parameter-less mints (e.g. mint(), claim(), publicMint())
        if func_sig.ends_with("()") {
            return Ok(data);
        }

        // Allowlist Merkle proof mints: bytes32[]
        if func_sig.contains("bytes32[]") {
            let proof_bytes = encode_bytes32_slice(&self.merkle_proof)?;
            if func_sig.contains("address") && func_sig.contains("uint256") {
                data.extend_from_slice(&encode_address_word(caller)?);
                data.extend_from_slice(&u256_word(quantity));
                data.extend_from_slice(&u256_word(96));
            } else if func_sig.contains("uint256") {
                data.extend_from_slice(&u256_word(quantity));
                data.extend_from_slice(&u256_word(64));
            } else if func_sig.contains("address") {
                data.extend_from_slice(&encode_address_word(caller)?);
                data.extend_from_slice(&u256_word(64));
            } else {
                data.extend_from_slice(&u256_word(32));
            }
            data.extend_from_slice(&proof_bytes);
            return Ok(data);
        }

        // Recipient mints: mint(address,uint256)
        if func_sig.contains("address,uint256") {
            let addr_bytes = encode_address_word(caller)?;
            data.extend_from_slice(&addr_bytes);
            data.extend_from_slice(&u256_word(quantity));
            return Ok(data);
        }

        // Quantity mints: mint(uint256)
        if func_sig.contains("uint256") && !func_sig.contains("address") {
            data.extend_from_slice(&u256_word(quantity));
            return Ok(data);
        }

        // Address only mints: mint(address)
        if func_sig.contains("address") && !func_sig.contains("uint256") {
            let addr_bytes = encode_address_word(caller)?;
            data.extend_from_slice(&addr_bytes);
            return Ok(data);
        }

        Ok(data)
    }

    fn seadrop_addrs(&self, minter: Option<&str>) -> Result<SeaDropAddrs, String> {
        let nft = alloy::primitives::Address::from_str(self.target_contract.trim())
            .map_err(|e| format!("Invalid target NFT address for SeaDrop: {e}"))?;
        let fee_str = self.seadrop_fee_recipient.as_deref().unwrap_or("0x0000a26b00c1f0df003000390027140000faa719");
        let fee = alloy::primitives::Address::from_str(fee_str.trim())
            .map_err(|e| format!("Invalid SeaDrop fee recipient: {e}"))?;
        let m = minter.ok_or_else(|| "SeaDrop calldata requires minter address".to_string())?;
        let minter_addr = alloy::primitives::Address::from_str(m.trim())
            .map_err(|e| format!("Invalid minter address for SeaDrop: {e}"))?;

        let mut w1 = [0u8; 32];
        w1[12..32].copy_from_slice(nft.as_slice());
        let mut w2 = [0u8; 32];
        w2[12..32].copy_from_slice(fee.as_slice());
        let mut w3 = [0u8; 32];
        w3[12..32].copy_from_slice(minter_addr.as_slice());
        Ok((w1, w2, w3))
    }

    /// Builds exact SeaDrop mintPublic calldata: mintPublic(address,address,address,uint256)
    pub fn build_seadrop_calldata(&self, minter: Option<&str>, quantity: u64) -> Result<Vec<u8>, String> {
        let (w1, w2, w3) = self.seadrop_addrs(minter)?;
        let mut data = Vec::with_capacity(4 + 128);
        data.extend_from_slice(&[0x16, 0x1a, 0xc2, 0x1f]);
        data.extend_from_slice(&w1);
        data.extend_from_slice(&w2);
        data.extend_from_slice(&w3);
        data.extend_from_slice(&u256_word(quantity));
        Ok(data)
    }

    /// Builds exact SeaDrop `mintAllowList` calldata (selector `0x4300a4e6`).
    /// `quantity` precedes the inlined `MintParams` tuple; proof offset is 13 words = 416 bytes.
    #[allow(clippy::too_many_arguments)]
    pub fn build_seadrop_allowlist_calldata(
        &self,
        minter: Option<&str>,
        quantity: u64,
        end_time: u64,
        drop_stage_index: u64,
        max_supply: u64,
        fee_bps: u16,
        wallet_limit: u64,
    ) -> Result<Vec<u8>, String> {
        let (w1, w2, w3) = self.seadrop_addrs(minter)?;
        let mint_price = self.mint_value_wei.trim().parse::<u128>()
            .map_err(|e| format!("Invalid mint_value_wei '{}': {e}", self.mint_value_wei))?;

        let mut data = Vec::with_capacity(4 + 32 * (14 + self.merkle_proof.len()));
        data.extend_from_slice(&crate::eligibility::seadrop::SEADROP_MINT_ALLOWLIST_SELECTOR);
        data.extend_from_slice(&w1);
        data.extend_from_slice(&w2);
        data.extend_from_slice(&w3);
        data.extend_from_slice(&u256_word(quantity));

        // MintParams tuple: 8 static words, inlined.
        let mut price_word = [0u8; 32];
        price_word[16..32].copy_from_slice(&mint_price.to_be_bytes());
        data.extend_from_slice(&price_word);
        let limit = if wallet_limit > 0 { wallet_limit } else { quantity };
        data.extend_from_slice(&u256_word(limit));
        data.extend_from_slice(&u256_word(self.target_timestamp.unwrap_or(0)));
        let final_end = if end_time > 0 { end_time } else { u64::MAX };
        data.extend_from_slice(&u256_word(final_end));
        data.extend_from_slice(&u256_word(drop_stage_index));
        data.extend_from_slice(&u256_word(max_supply));
        data.extend_from_slice(&u256_word(fee_bps as u64));
        data.extend_from_slice(&u256_word(0)); // restrictFeeRecipients (bool)

        data.extend_from_slice(&u256_word(416)); // proof offset: 13 * 32 = 416
        data.extend_from_slice(&encode_bytes32_slice(&self.merkle_proof)?);
        Ok(data)
    }

    /// Resolves calldata based on whether SeaDrop is used (public or allowlist) or native contract.
    pub fn build_dispatch_calldata(&self, caller: Option<&str>, quantity: u64) -> Result<Vec<u8>, String> {
        if self.seadrop_contract.is_some() {
            if !self.merkle_proof.is_empty() {
                self.build_seadrop_allowlist_calldata(
                    caller,
                    quantity,
                    self.allowlist_end_time.unwrap_or(u64::MAX),
                    self.allowlist_stage_index.unwrap_or(0),
                    self.allowlist_max_supply.unwrap_or(0),
                    self.allowlist_fee_bps.unwrap_or(0),
                    self.allowlist_wallet_limit.unwrap_or(quantity),
                )
            } else {
                self.build_seadrop_calldata(caller, quantity)
            }
        } else {
            self.build_calldata_for_caller(caller, quantity)
        }
    }

    /// Resolves the on-chain destination contract address for the transaction (SeaDrop contract if active, or target NFT contract).
    pub fn get_dispatch_destination<'a>(&'a self, fallback_target: &'a str) -> &'a str {
        if let Some(ref sd) = self.seadrop_contract {
            sd.as_str()
        } else {
            fallback_target
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasConfig {
    pub max_fee_gwei: f64,
    pub max_priority_fee_gwei: f64,
    pub gas_limit: u64,
    pub auto_speedup: bool,
    pub speedup_threshold_ms: u64,
    pub speedup_bump_percent: u32, // default: 15%
    pub hard_gas_ceiling_gwei: f64,
    #[serde(default)]
    pub allow_public_fallback: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertConfig {
    pub discord_webhook: Option<String>,
    pub telegram_bot_token: Option<String>,
    pub telegram_chat_id: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            chain: ChainConfig {
                chain_id: 1,
                name: "Ethereum Mainnet".to_string(),
                rpc_urls: vec!["https://eth.llamarpc.com".to_string(), "https://rpc.payload.de".to_string()],
                ws_rpc_url: None,
                mev_builder_urls: vec!["https://relay.flashbots.net".to_string(), "https://rpc.titanbuilder.xyz".to_string()],
                rpc_timeout_ms: 2000,
            },
            drop: DropConfig::default(),
            gas: GasConfig {
                max_fee_gwei: 50.0,
                max_priority_fee_gwei: 3.0,
                gas_limit: 150_000,
                auto_speedup: true,
                speedup_threshold_ms: 12_000,
                speedup_bump_percent: 15,
                hard_gas_ceiling_gwei: 150.0,
                allow_public_fallback: false,
            },
            alerts: AlertConfig { discord_webhook: None, telegram_bot_token: None, telegram_chat_id: None },
        }
    }
}

impl AppConfig {
    pub fn load_from_file(path: &Path) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let content = fs::read_to_string(path)?;
        Ok(toml::from_str(&content)?)
    }

    pub fn save_to_file(&self, path: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let content = toml::to_string_pretty(self)?;
        if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
            std::io::Write::write_all(&mut file, content.as_bytes())?;
        }
        #[cfg(not(unix))]
        { fs::write(path, content)?; }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calldata_builder_parameterless() {
        let drop = DropConfig { mint_function: "mint()".to_string(), ..Default::default() };
        assert_eq!(drop.build_calldata().unwrap().len(), 4);
    }

    #[test]
    fn test_calldata_builder_uint256() {
        let drop = DropConfig { mint_function: "mint(uint256)".to_string(), ..Default::default() };
        assert_eq!(drop.build_calldata().unwrap().len(), 36);
        assert_eq!(drop.build_calldata_for_caller(None, 3).unwrap()[35], 3);
    }

    #[test]
    fn test_calldata_builder_address_uint256() {
        let drop = DropConfig { mint_function: "mint(address,uint256)".to_string(), ..Default::default() };
        let caller = "0x1111111111111111111111111111111111111111";
        let calldata = drop.build_calldata_for_caller(Some(caller), 1).unwrap();
        assert_eq!(calldata.len(), 4 + 32 + 32);
        assert_eq!(calldata[67], 1);
    }

    #[test]
    fn test_seadrop_selectors_match_constants() {
        use alloy::primitives::keccak256;
        assert_eq!(&keccak256("getPublicDrop(address)")[0..4], &[0xbc, 0x6a, 0x62, 0x9c]);
        assert_eq!(&keccak256("getMintStats(address)")[0..4], &[0x84, 0x0e, 0x15, 0xd4]);
        assert_eq!(&keccak256("getAllowedFeeRecipients(address)")[0..4], &[0x68, 0x63, 0x22, 0x74]);
        assert_eq!(&keccak256("getAllowListMerkleRoot(address)")[0..4], &[0x32, 0xbf, 0x11, 0xf5]);
        assert_eq!(&keccak256("mintPublic(address,address,address,uint256)")[0..4], &[0x16, 0x1a, 0xc2, 0x1f]);
        assert_eq!(
            &keccak256(
                "mintAllowList(address,address,address,uint256,(uint256,uint256,uint256,uint256,uint256,uint256,uint256,bool),bytes32[])"
            )[0..4],
            &[0x43, 0x00, 0xa4, 0xe6]
        );
        assert_eq!(
            crate::eligibility::seadrop::SEADROP_MINT_ALLOWLIST_SELECTOR,
            [0x43, 0x00, 0xa4, 0xe6]
        );
    }

    #[test]
    fn test_calldata_builder_merkle_proof() {
        let proof_elem = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string();
        let drop = DropConfig {
            mint_function: "mint(uint256,bytes32[])".to_string(),
            merkle_proof: vec![proof_elem.clone()],
            ..Default::default()
        };
        let calldata = drop.build_calldata().unwrap();
        // 4 (sel) + 32 (qty) + 32 (offset: 64) + 32 (len: 1) + 32 (proof) = 132 bytes
        assert_eq!(calldata.len(), 4 + 32 * 4);
        assert_eq!(calldata[35], 1); // quantity
        assert_eq!(calldata[67], 64); // offset pointer 0x40
        assert_eq!(calldata[99], 1); // length of proof

        let drop_no_qty = DropConfig {
            mint_function: "whitelistMint(bytes32[])".to_string(),
            merkle_proof: vec![proof_elem],
            ..Default::default()
        };
        let calldata_no_qty = drop_no_qty.build_calldata().unwrap();
        // 4 + 32 (offset: 32) + 32 (len: 1) + 32 (proof) = 100 bytes
        assert_eq!(calldata_no_qty.len(), 4 + 32 * 3);
        assert_eq!(calldata_no_qty[35], 32); // offset 0x20
    }

    #[test]
    fn test_seadrop_allowlist_calldata() {
        let proof_elem = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string();
        let drop = DropConfig {
            target_contract: "0x2222222222222222222222222222222222222222".to_string(),
            seadrop_contract: Some(crate::eligibility::seadrop::SEADROP_V1_ADDRESS.to_string()),
            mint_value_wei: "50000000000000000".to_string(),
            merkle_proof: vec![proof_elem],
            allowlist_end_time: Some(1_800_000_000),
            allowlist_stage_index: Some(1),
            allowlist_max_supply: Some(500),
            allowlist_fee_bps: Some(250),
            allowlist_wallet_limit: Some(5),
            ..Default::default()
        };
        let caller = "0x1111111111111111111111111111111111111111";
        let calldata = drop.build_dispatch_calldata(Some(caller), 2).unwrap();
        assert_eq!(&calldata[0..4], &[0x43, 0x00, 0xa4, 0xe6]);
        // selector + 4 top-level words + 8 tuple words + offset + length + 1 proof
        assert_eq!(calldata.len(), 4 + 32 * 15);

        // Word 3: top-level `quantity`
        assert_eq!(calldata[4 + 3 * 32 + 31], 2);
        // Word 5: maxTotalMintableByWallet = 5
        assert_eq!(calldata[4 + 5 * 32 + 31], 5);
        // Word 7: endTime = 1_800_000_000
        let end_idx = 4 + 7 * 32;
        assert_eq!(u64::from_be_bytes(calldata[end_idx + 24..end_idx + 32].try_into().unwrap()), 1_800_000_000);
        // Word 8: dropStageIndex = 1
        assert_eq!(calldata[4 + 8 * 32 + 31], 1);
        // Word 10: feeBps = 250
        assert_eq!(calldata[4 + 10 * 32 + 31], 250);
        // Word 12 is the offset to bytes32[]: 416 (0x01a0)
        let offset_idx = 4 + 12 * 32 + 30;
        assert_eq!(calldata[offset_idx], 1);
        assert_eq!(calldata[offset_idx + 1], 0xa0);
    }
}

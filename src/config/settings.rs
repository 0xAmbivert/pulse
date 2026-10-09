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
    pub mev_builder_urls: Vec<String>,
    pub rpc_timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropConfig {
    pub target_contract: String,
    pub mint_function: String,
    pub mint_value_wei: String,
    pub custom_calldata_hex: Option<String>,
    pub target_timestamp: Option<u64>,
    pub target_block: Option<u64>,
    pub monitor_owner_address: Option<String>,
    pub flip_function_signatures: Vec<String>,
}

impl Default for DropConfig {
    fn default() -> Self {
        Self {
            target_contract: "0x0000000000000000000000000000000000000000".to_string(),
            mint_function: "mint(uint256)".to_string(),
            mint_value_wei: "0".to_string(),
            custom_calldata_hex: None,
            target_timestamp: None,
            target_block: None,
            monitor_owner_address: None,
            flip_function_signatures: vec![
                "isPublicSaleActive()".to_string(),
                "publicSaleActive()".to_string(),
                "saleIsActive()".to_string(),
            ],
        }
    }
}

impl DropConfig {
    pub fn build_calldata(&self) -> Result<Vec<u8>, String> {
        self.build_calldata_for_caller(None)
    }

    pub fn build_calldata_for_caller(&self, caller: Option<&str>) -> Result<Vec<u8>, String> {
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

        // Recipient mints: mint(address,uint256)
        if func_sig.contains("address,uint256") {
            let mut addr_bytes = [0u8; 32];
            if let Some(c) = caller {
                if let Ok(addr) = alloy::primitives::Address::from_str(c.trim()) {
                    addr_bytes[12..32].copy_from_slice(addr.as_slice());
                }
            }
            data.extend_from_slice(&addr_bytes);
            let mut amount = [0u8; 32];
            amount[31] = 1;
            data.extend_from_slice(&amount);
            return Ok(data);
        }

        // Quantity mints: mint(uint256)
        if func_sig.contains("uint256") && !func_sig.contains("address") {
            let mut amount = [0u8; 32];
            amount[31] = 1;
            data.extend_from_slice(&amount);
            return Ok(data);
        }

        // Address only mints: mint(address)
        if func_sig.contains("address") && !func_sig.contains("uint256") {
            let mut addr_bytes = [0u8; 32];
            if let Some(c) = caller {
                if let Ok(addr) = alloy::primitives::Address::from_str(c.trim()) {
                    addr_bytes[12..32].copy_from_slice(addr.as_slice());
                }
            }
            data.extend_from_slice(&addr_bytes);
            return Ok(data);
        }

        Ok(data)
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
                rpc_urls: vec![
                    "https://eth.llamarpc.com".to_string(),
                    "https://rpc.payload.de".to_string(),
                ],
                mev_builder_urls: vec![
                    "https://relay.flashbots.net".to_string(),
                    "https://rpc.titanbuilder.xyz".to_string(),
                ],
                rpc_timeout_ms: 2000,
            },
            drop: DropConfig {
                target_contract: "0x0000000000000000000000000000000000000000".to_string(),
                mint_function: "mint(uint256)".to_string(),
                mint_value_wei: "0".to_string(),
                custom_calldata_hex: None,
                target_timestamp: None,
                target_block: None,
                monitor_owner_address: None,
                flip_function_signatures: vec![
                    "isPublicSaleActive()".to_string(),
                    "publicSaleActive()".to_string(),
                    "saleIsActive()".to_string(),
                ],
            },
            gas: GasConfig {
                max_fee_gwei: 50.0,
                max_priority_fee_gwei: 3.0,
                gas_limit: 150_000,
                auto_speedup: true,
                speedup_threshold_ms: 12_000,
                speedup_bump_percent: 15,
                hard_gas_ceiling_gwei: 150.0,
            },
            alerts: AlertConfig {
                discord_webhook: None,
                telegram_bot_token: None,
                telegram_chat_id: None,
            },
        }
    }
}

impl AppConfig {
    pub fn load_from_file(path: &Path) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let content = fs::read_to_string(path)?;
        let config: AppConfig = toml::from_str(&content)?;
        Ok(config)
    }

    pub fn save_to_file(&self, path: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let content = toml::to_string_pretty(self)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(path)?;

            // Explicitly set permissions on existing files just in case
            if let Ok(metadata) = file.metadata() {
                let mut perms = metadata.permissions();
                perms.set_mode(0o600);
                let _ = file.set_permissions(perms);
            }

            std::io::Write::write_all(&mut file, content.as_bytes())?;
        }
        
        #[cfg(not(unix))]
        {
            fs::write(path, content)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calldata_builder_parameterless() {
        let drop = DropConfig {
            mint_function: "mint()".to_string(),
            ..Default::default()
        };
        let calldata = drop.build_calldata().unwrap();
        assert_eq!(calldata.len(), 4);
    }

    #[test]
    fn test_calldata_builder_uint256() {
        let drop = DropConfig {
            mint_function: "mint(uint256)".to_string(),
            ..Default::default()
        };
        let calldata = drop.build_calldata().unwrap();
        assert_eq!(calldata.len(), 36);
        assert_eq!(calldata[35], 1);
    }

    #[test]
    fn test_calldata_builder_address_uint256() {
        let drop = DropConfig {
            mint_function: "mint(address,uint256)".to_string(),
            ..Default::default()
        };
        let caller = "0x1111111111111111111111111111111111111111";
        let calldata = drop.build_calldata_for_caller(Some(caller)).unwrap();
        assert_eq!(calldata.len(), 4 + 32 + 32);
        assert_eq!(calldata[67], 1); // quantity 1 at end of second 32-byte word
    }
}

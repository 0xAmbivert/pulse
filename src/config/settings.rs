use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub chain: ChainConfig,
    pub drop: DropConfig,
    pub gas: GasConfig,
    pub wallets: WalletConfig,
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
pub struct WalletConfig {
    pub keystore_paths: Vec<String>,
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
                    "setPublicSaleActive(bool)".to_string(),
                    "flipSaleState()".to_string(),
                    "unpause()".to_string(),
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
            wallets: WalletConfig {
                keystore_paths: vec!["./keystores/wallet_01.json".to_string()],
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
        fs::write(path, content)?;
        Ok(())
    }
}

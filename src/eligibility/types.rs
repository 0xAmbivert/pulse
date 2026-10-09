use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedDropTarget {
    pub contract_address: String,
    pub collection_name: String,
    pub collection_slug: Option<String>,
    pub chain: String,
    pub chain_id: u64,
    pub opensea_url: Option<String>,
    pub fee_recipient: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MintStageInfo {
    pub id: String,
    pub stage_type: String, // "public_sale", "signed_presale", "allowlist", etc.
    pub label: String,
    pub price_wei: u128,
    pub price_eth: f64,
    pub start_time_unix: Option<u64>,
    pub end_time_unix: Option<u64>,
    pub max_per_wallet: Option<u64>,
    pub is_active_now: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeaDropPublicConfig {
    pub mint_price_wei: u128,
    pub start_time: u64,
    pub end_time: u64,
    pub max_per_wallet: u64,
    pub fee_bps: u16,
    pub fee_recipient: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletEligibilityStatus {
    pub wallet_address: String,
    pub is_eligible: bool,
    pub remaining_mintable: u64,
    pub minted_by_wallet: u64,
    pub max_per_wallet: u64,
    pub price_wei: u128,
    pub price_eth: f64,
    pub status_reason: String,
    pub simulated_gas_used: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComprehensiveDropReport {
    pub target: ResolvedDropTarget,
    pub minted_supply: u64,
    pub max_supply: Option<u64>,
    pub is_sold_out: bool,
    pub drop_mechanism: String, // "SeaDrop v1.0", "Custom Contract", "Unknown"
    pub stages: Vec<MintStageInfo>,
    pub seadrop_config: Option<SeaDropPublicConfig>,
    pub wallet_status: WalletEligibilityStatus,
}

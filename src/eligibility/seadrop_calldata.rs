//! Canonical SeaDrop v1 `mintAllowList` calldata construction.
//!
//! # Why this module exists
//!
//! `DropConfig::build_seadrop_allowlist_calldata` previously emitted a 4-byte selector of
//! `0x7b233a75`. That constant corresponds to **no** function on the deployed SeaDrop
//! contract; it is absent from the 4byte signature database. The real signature is:
//!
//! ```text
//! mintAllowList(address,address,address,uint256,
//!               (uint256,uint256,uint256,uint256,uint256,uint256,uint256,bool),
//!               bytes32[])
//! ```
//!
//! which hashes to `0x4300a4e6`. Calling with the wrong selector makes the EVM dispatch
//! to the contract's fallback, so every allowlist mint reverted and only burned gas,
//! silently, no matter how valid the Merkle proof was.
//!
//! The same function also mis-ordered its arguments: it inlined the `MintParams` tuple
//! where the top-level `quantity` word belongs, shifting every subsequent argument by one
//! word and desynchronising the `bytes32[]` offset. This module encodes the layout that
//! the contract actually decodes, and is covered by `tests/seadrop_abi.rs`.
//!
//! Argument block layout (32-byte words, offsets measured from the start of the argument
//! block, i.e. immediately after the 4-byte selector):
//!
//! | word | field                      |
//! |------|----------------------------|
//! | 0    | `nftContract`              |
//! | 1    | `feeRecipient`             |
//! | 2    | `minterIfNotPayer`         |
//! | 3    | `quantity`                 |
//! | 4    | `mintParams.mintPrice`     |
//! | 5    | `maxTotalMintableByWallet` |
//! | 6    | `startTime`                |
//! | 7    | `endTime`                  |
//! | 8    | `dropStageIndex`           |
//! | 9    | `maxTokenSupplyForStage`   |
//! | 10   | `feeBps`                   |
//! | 11   | `restrictFeeRecipients`    |
//! | 12   | offset to `proof` (== 416) |
//! | 13   | `proof.length`             |
//! | 14.. | proof elements             |

use crate::config::settings::DropConfig;
use crate::eligibility::seadrop::SEADROP_MINT_ALLOWLIST_SELECTOR;
use alloy::primitives::Address;
use std::str::FromStr;

/// Number of static argument words that precede the `bytes32[] proof` offset word:
/// 4 top-level words (`nftContract`, `feeRecipient`, `minterIfNotPayer`, `quantity`) plus the
/// 8 words of the inlined static `MintParams` tuple.
const STATIC_WORDS_BEFORE_PROOF: usize = 12;

/// Byte offset of the `bytes32[] proof` data from the start of the argument block.
/// The offset word itself occupies word 12, so the proof begins at word 13 (`416` bytes).
const PROOF_DATA_OFFSET: usize = (STATIC_WORDS_BEFORE_PROOF + 1) * 32;

fn u256_word(value: u128) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[16..32].copy_from_slice(&value.to_be_bytes());
    word
}

fn address_word(input: &str, label: &str) -> Result<[u8; 32], String> {
    let addr = Address::from_str(input.trim())
        .map_err(|e| format!("Invalid {label} address '{input}': {e}"))?;
    let mut word = [0u8; 32];
    word[12..32].copy_from_slice(addr.as_slice());
    Ok(word)
}

/// Decodes one 32-byte proof element from hex, tolerating an optional `0x` prefix.
fn proof_element(entry: &str, index: usize) -> Result<[u8; 32], String> {
    let clean = entry.trim().trim_start_matches("0x");
    if clean.len() != 64 {
        return Err(format!(
            "Merkle proof element {index} must be 32 bytes (64 hex characters), got {} characters: '{entry}'",
            clean.len()
        ));
    }
    let decoded = hex::decode(clean)
        .map_err(|e| format!("Invalid hex in Merkle proof element {index}: {e}"))?;
    let mut out = [0u8; 32];
    out.copy_from_slice(&decoded);
    Ok(out)
}

/// Parses `mint_value_wei`, which holds the **per-token** unit price.
fn unit_price_wei(config: &DropConfig) -> Result<u128, String> {
    config
        .mint_value_wei
        .trim()
        .parse::<u128>()
        .map_err(|e| format!("Invalid mint_value_wei '{}': {e}", config.mint_value_wei))
}

/// Total `msg.value` for a dispatch of `quantity` tokens.
///
/// SeaDrop enforces `msg.value == quantity * mintPrice` inside `_checkCorrectPayment`.
/// Sending the bare unit price with `quantity > 1` reverts with `IncorrectPayment`.
pub fn total_dispatch_value_wei(config: &DropConfig, quantity: u64) -> Result<u128, String> {
    let unit = unit_price_wei(config)?;
    unit.checked_mul(quantity as u128).ok_or_else(|| {
        format!("Mint value overflow: {unit} wei x quantity {quantity} exceeds u128")
    })
}

/// Encodes `mintAllowList(...)` calldata for the configured allowlist stage.
pub fn encode_mint_allow_list(
    config: &DropConfig,
    minter: &str,
    quantity: u64,
) -> Result<Vec<u8>, String> {
    if quantity == 0 {
        return Err("Mint quantity must be greater than zero".to_string());
    }

    let fee_recipient = config
        .seadrop_fee_recipient
        .as_deref()
        .unwrap_or("0x0000a26b00c1f0df003000390027140000faa719");

    // MintParams words.
    let mint_price = unit_price_wei(config)?;
    let wallet_limit = match config.allowlist_wallet_limit {
        Some(limit) if limit > 0 => limit,
        _ => quantity,
    };
    let start_time = config.target_timestamp.unwrap_or(0);
    let end_time = config.allowlist_end_time.unwrap_or(u64::MAX);
    // Default to stage 1 when not explicitly specified, since OpenSea/SeaDrop uses
    // dropStageIndex = 1 for the primary allowlist stage. Explicit 0 is rejected.
    let stage_index = config.allowlist_stage_index.unwrap_or(1);
    if stage_index == 0 {
        return Err(
            "SeaDrop rejects dropStageIndex 0 for allowlist stages (must be non-zero)".to_string(),
        );
    }
    let max_token_supply = config.allowlist_max_supply.unwrap_or(0);
    let fee_bps = config.allowlist_fee_bps.unwrap_or(0);

    let mut data = Vec::with_capacity(
        4 + 32 * (STATIC_WORDS_BEFORE_PROOF + 2 + config.merkle_proof.len()),
    );
    data.extend_from_slice(&SEADROP_MINT_ALLOWLIST_SELECTOR);

    // Top-level arguments, in declaration order.
    data.extend_from_slice(&address_word(&config.target_contract, "nftContract")?);
    data.extend_from_slice(&address_word(fee_recipient, "feeRecipient")?);
    data.extend_from_slice(&address_word(minter, "minterIfNotPayer")?);
    data.extend_from_slice(&u256_word(quantity as u128));

    // MintParams tuple: all fields are static, so the tuple is inlined in place.
    data.extend_from_slice(&u256_word(mint_price));
    data.extend_from_slice(&u256_word(wallet_limit as u128));
    data.extend_from_slice(&u256_word(start_time as u128));
    data.extend_from_slice(&u256_word(end_time as u128));
    data.extend_from_slice(&u256_word(stage_index as u128));
    data.extend_from_slice(&u256_word(max_token_supply as u128));
    data.extend_from_slice(&u256_word(fee_bps as u128));
    data.extend_from_slice(&u256_word(0)); // restrictFeeRecipients = false

    // Offset to the dynamic `bytes32[] proof`, measured from the argument block start.
    data.extend_from_slice(&u256_word(PROOF_DATA_OFFSET as u128));

    // `bytes32[] proof`.
    data.extend_from_slice(&u256_word(config.merkle_proof.len() as u128));
    for (index, entry) in config.merkle_proof.iter().enumerate() {
        data.extend_from_slice(&proof_element(entry, index)?);
    }

    Ok(data)
}

/// Encodes the calldata for one dispatch, routing SeaDrop allowlist mints through the
/// canonical `mintAllowList` encoder and everything else through the generic builder.
pub fn encode_dispatch_calldata(
    config: &DropConfig,
    minter: &str,
    quantity: u64,
) -> Result<Vec<u8>, String> {
    if config.seadrop_contract.is_some() && !config.merkle_proof.is_empty() {
        return encode_mint_allow_list(config, minter, quantity);
    }
    config.build_dispatch_calldata(Some(minter), quantity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eligibility::seadrop::SEADROP_V1_ADDRESS;

    fn allowlist_config() -> DropConfig {
        DropConfig {
            target_contract: "0x2222222222222222222222222222222222222222".to_string(),
            seadrop_contract: Some(SEADROP_V1_ADDRESS.to_string()),
            mint_value_wei: "1000".to_string(),
            merkle_proof: vec![
                "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            ],
            allowlist_end_time: Some(1_800_000_000),
            allowlist_stage_index: Some(1),
            allowlist_max_supply: Some(500),
            allowlist_fee_bps: Some(250),
            allowlist_wallet_limit: Some(5),
            target_timestamp: Some(1_700_000_000),
            ..Default::default()
        }
    }

    /// Reads the low 16 bytes of argument word `index` (all values used here fit in u128).
    fn word(calldata: &[u8], index: usize) -> u128 {
        let start = 4 + index * 32;
        let mut buf = [0u8; 16];
        buf.copy_from_slice(&calldata[start + 16..start + 32]);
        u128::from_be_bytes(buf)
    }

    #[test]
    fn encodes_selector_and_quantity_position() {
        let cfg = allowlist_config();
        let data = encode_mint_allow_list(&cfg, "0x1111111111111111111111111111111111111111", 2).unwrap();

        assert_eq!(&data[0..4], &SEADROP_MINT_ALLOWLIST_SELECTOR);
        assert_eq!(&data[0..4], &[0x43, 0x00, 0xa4, 0xe6]);
        // quantity is a top-level argument at word 3, before the tuple
        assert_eq!(word(&data, 3), 2);
        // total length: selector + 12 static + offset + length + 1 proof
        assert_eq!(data.len(), 4 + 32 * 15);
    }

    #[test]
    fn encodes_mint_params_word_order() {
        let cfg = allowlist_config();
        let data = encode_mint_allow_list(&cfg, "0x1111111111111111111111111111111111111111", 2).unwrap();

        assert_eq!(word(&data, 4), 1000, "mintPrice is the unit price");
        assert_eq!(word(&data, 5), 5, "maxTotalMintableByWallet");
        assert_eq!(word(&data, 6), 1_700_000_000, "startTime");
        assert_eq!(word(&data, 7), 1_800_000_000, "endTime");
        assert_eq!(word(&data, 8), 1, "dropStageIndex");
        assert_eq!(word(&data, 9), 500, "maxTokenSupplyForStage");
        assert_eq!(word(&data, 10), 250, "feeBps");
        assert_eq!(word(&data, 11), 0, "restrictFeeRecipients");
    }

    #[test]
    fn proof_offset_points_at_proof_length_word() {
        let cfg = allowlist_config();
        let data = encode_mint_allow_list(&cfg, "0x1111111111111111111111111111111111111111", 2).unwrap();

        assert_eq!(word(&data, 12), 416, "offset must skip the 12 static words plus itself");
        // Follow the offset to confirm it lands on the array length word.
        let offset = word(&data, 12) as usize;
        let len_start = 4 + offset;
        let mut len_buf = [0u8; 16];
        len_buf.copy_from_slice(&data[len_start + 16..len_start + 32]);
        let len = u128::from_be_bytes(len_buf);
        assert_eq!(len, 1, "one proof element");
        // And the element must be the configured hash.
        let element_start = len_start + 32;
        assert_eq!(
            hex::encode(&data[element_start..element_start + 32]),
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        );
    }

    #[test]
    fn rejects_zero_stage_index_and_bad_proof() {
        let mut cfg = allowlist_config();
        cfg.allowlist_stage_index = Some(0);
        assert!(encode_mint_allow_list(&cfg, "0x1111111111111111111111111111111111111111", 1).is_err());

        // When None (unspecified in CLI menu or config), defaults to stage 1.
        let mut cfg_default_stage = allowlist_config();
        cfg_default_stage.allowlist_stage_index = None;
        let encoded = encode_mint_allow_list(&cfg_default_stage, "0x1111111111111111111111111111111111111111", 1).unwrap();
        assert_eq!(word(&encoded, 8), 1, "unspecified stage index must default to 1");

        let mut cfg = allowlist_config();
        cfg.merkle_proof = vec!["0xdeadbeef".to_string()];
        assert!(encode_mint_allow_list(&cfg, "0x1111111111111111111111111111111111111111", 1).is_err());
    }

    #[test]
    fn total_value_scales_with_quantity() {
        let cfg = allowlist_config();
        assert_eq!(total_dispatch_value_wei(&cfg, 1).unwrap(), 1000);
        assert_eq!(total_dispatch_value_wei(&cfg, 3).unwrap(), 3000);
    }

    #[test]
    fn total_value_reports_overflow_instead_of_wrapping() {
        let mut cfg = allowlist_config();
        cfg.mint_value_wei = u128::MAX.to_string();
        assert!(total_dispatch_value_wei(&cfg, 2).is_err());
    }
}

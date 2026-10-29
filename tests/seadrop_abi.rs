//! Regression tests pinning the SeaDrop ABI surface to the canonical
//! `ProjectOpenSea/seadrop` definitions.
//!
//! These exist because the codebase previously shipped `mintAllowList` calls against a
//! selector that corresponds to no deployed SeaDrop function
//! (`0x7b233a75`, absent from the 4byte signature database), which made every
//! allowlist mint revert and burn gas regardless of proof validity.

use alloy::primitives::keccak256;

fn selector(sig: &str) -> [u8; 4] {
    let h = keccak256(sig.as_bytes());
    [h[0], h[1], h[2], h[3]]
}

/// Canonical signature, verified against the deployed SeaDrop contract at
/// `0x00005EA00Ac477B1030CE78506496e8C2dE24bf5`.
const MINT_ALLOWLIST_SIG: &str = "mintAllowList(address,address,address,uint256,(uint256,uint256,uint256,uint256,uint256,uint256,uint256,bool),bytes32[])";
const MINT_PUBLIC_SIG: &str = "mintPublic(address,address,address,uint256)";

#[test]
fn mint_allowlist_selector_matches_canonical_signature() {
    assert_eq!(selector(MINT_ALLOWLIST_SIG), [0x43, 0x00, 0xa4, 0xe6]);
    assert_eq!(
        selector(MINT_ALLOWLIST_SIG),
        pulse::eligibility::seadrop::SEADROP_MINT_ALLOWLIST_SELECTOR
    );
}

#[test]
fn mint_public_selector_matches_canonical_signature() {
    assert_eq!(selector(MINT_PUBLIC_SIG), [0x16, 0x1a, 0xc2, 0x1f]);
    assert_eq!(
        selector(MINT_PUBLIC_SIG),
        pulse::eligibility::seadrop::SEADROP_MINT_PUBLIC_SELECTOR
    );
}

#[test]
fn legacy_bogus_allowlist_selector_is_rejected() {
    // The selector that used to be hardcoded as `mintAllowList`. It maps to no known
    // function signature, so its presence anywhere in the dispatch path is a bug.
    let bogus = [0x7b, 0x23, 0x3a, 0x75];
    assert_ne!(selector(MINT_ALLOWLIST_SIG), bogus);
    assert_ne!(
        pulse::eligibility::seadrop::SEADROP_MINT_ALLOWLIST_SELECTOR,
        bogus
    );
}

/// Documents the required argument layout of the allowlist dispatch.
///
/// `mintAllowList(address,address,address,uint256,MintParams,bytes32[])` has 12 static
/// words (nftContract, feeRecipient, minterIfNotPayer, quantity, then the 8 inlined
/// `MintParams` words) plus 1 offset word for `bytes32[] proof` in the head section,
/// so the `bytes32[] proof` tail begins at `(12 + 1) * 32 == 416` (`0x1a0`).
/// `quantity` is a top-level argument and must precede the tuple.
#[test]
fn expected_allowlist_word_layout() {
    // word index -> meaning
    let layout = [
        "nftContract",
        "feeRecipient",
        "minterIfNotPayer",
        "quantity",
        "mintPrice",
        "maxTotalMintableByWallet",
        "startTime",
        "endTime",
        "dropStageIndex",
        "maxTokenSupplyForStage",
        "feeBps",
        "restrictFeeRecipients",
        "proofOffset", // == 416 (13 head words * 32 bytes)
        "proofLength",
        // proof elements follow
    ];
    assert_eq!(layout.len(), 14);
    let proof_offset = (layout.len() - 1) * 32;
    assert_eq!(proof_offset, 416, "proof offset is 13 head words (416 bytes)");
}

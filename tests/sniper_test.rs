use pulse::sniper::MempoolScanner;
use alloy::primitives::keccak256;

#[test]
fn test_mempool_scanner_selector_match() {
    let signatures = vec![
        "setPublicSaleActive(bool)".to_string(),
        "flipSaleState()".to_string(),
        "unpause()".to_string(),
    ];
    let scanner = MempoolScanner::new(
        "http://localhost:8545",
        "0x1111111111111111111111111111111111111111",
        None,
        &signatures,
    );

    // Verify flipSaleState selector
    let hash = keccak256("flipSaleState()".as_bytes());
    let flip_calldata = format!("0x{}", hex::encode(&hash[0..4]));
    assert!(scanner.matches_selector(&flip_calldata));

    // Verify unpause selector
    let unpause_hash = keccak256("unpause()".as_bytes());
    let unpause_calldata = format!("0x{}", hex::encode(&unpause_hash[0..4]));
    assert!(scanner.matches_selector(&unpause_calldata));

    // Verify non-matching random selector
    assert!(!scanner.matches_selector("0xdeadbeef"));
    assert!(!scanner.matches_selector("0x12")); // too short
}

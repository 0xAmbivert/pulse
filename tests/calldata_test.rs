use pulse::config::settings::DropConfig;

#[test]
fn test_calldata_construction_custom_hex() {
    let drop = DropConfig {
        target_contract: "0x1234567890123456789012345678901234567890".to_string(),
        mint_function: "mint(uint256)".to_string(),
        mint_value_wei: "0".to_string(),
        custom_calldata_hex: Some("0x12345678".to_string()),
        target_timestamp: None,
        target_block: None,
        monitor_owner_address: None,
        flip_function_signatures: vec![],
    };

    let calldata = drop.build_calldata().expect("valid calldata");
    assert_eq!(calldata, vec![0x12, 0x34, 0x56, 0x78]);
}

#[test]
fn test_calldata_construction_uint256_function() {
    let drop = DropConfig {
        target_contract: "0x1234567890123456789012345678901234567890".to_string(),
        mint_function: "mint(uint256)".to_string(),
        mint_value_wei: "0".to_string(),
        custom_calldata_hex: None,
        target_timestamp: None,
        target_block: None,
        monitor_owner_address: None,
        flip_function_signatures: vec![],
    };

    let calldata = drop.build_calldata().expect("valid calldata");
    assert_eq!(calldata.len(), 4 + 32);
    // Selector for mint(uint256) is 0xa0712d68
    assert_eq!(&calldata[0..4], &[0xa0, 0x71, 0x2d, 0x68]);
    // Trailing byte is quantity 1
    assert_eq!(calldata[35], 1);
}

#[test]
fn test_calldata_construction_no_args_function() {
    let drop = DropConfig {
        target_contract: "0x1234567890123456789012345678901234567890".to_string(),
        mint_function: "mint()".to_string(),
        mint_value_wei: "0".to_string(),
        custom_calldata_hex: None,
        target_timestamp: None,
        target_block: None,
        monitor_owner_address: None,
        flip_function_signatures: vec![],
    };

    let calldata = drop.build_calldata().expect("valid calldata");
    // Only 4 bytes selector
    assert_eq!(calldata.len(), 4);
    let hash = alloy::primitives::keccak256("mint()".as_bytes());
    assert_eq!(&calldata[0..4], &hash[0..4]);
}

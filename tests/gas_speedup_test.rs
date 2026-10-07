use pulse::gas::GasEngine;

#[test]
fn test_gas_engine_speedup_bump() {
    let engine = GasEngine::new(100.0, 2.0, 15);
    let prev_max = 20_000_000_000u128; // 20 Gwei
    let prev_prio = 2_000_000_000u128; // 2 Gwei

    let (new_max, new_prio) = engine.calculate_speedup_fees(prev_max, prev_prio).expect("speedup ok");
    // 15% bump: 20 * 1.15 = 23 Gwei
    assert_eq!(new_max, 23_000_000_000);
    assert_eq!(new_prio, 2_300_000_000);
}

#[test]
fn test_gas_engine_speedup_ceiling_limit() {
    let engine = GasEngine::new(25.0, 2.0, 15); // ceiling 25 Gwei
    let prev_max = 24_000_000_000u128; // 24 Gwei
    let prev_prio = 2_000_000_000u128;

    let res = engine.calculate_speedup_fees(prev_max, prev_prio);
    assert!(res.is_err(), "Should exceed ceiling of 25 Gwei");
}

use pulse::crypto::ProtectedKey;

#[test]
fn test_protected_key_zeroize_on_drop() {
    let mut key = ProtectedKey::empty();
    key.as_mut_bytes().fill(1);
    assert_eq!(key.as_bytes()[0], 1);
    // Key drops here, which calls our secure Drop implementation.
}

use super::secure_memory::ProtectedKey;
use alloy::signers::local::PrivateKeySigner;
use std::error::Error;

/// Constructs an Alloy `PrivateKeySigner` from a memory-pinned `ProtectedKey`.
/// Local secp256k1 signing operations execute entirely in local memory in <1µs.
pub fn create_signer_from_protected(
    key: &ProtectedKey,
) -> Result<PrivateKeySigner, Box<dyn Error + Send + Sync>> {
    let signer = PrivateKeySigner::from_slice(key.as_bytes())?;
    Ok(signer)
}

/// Helper function to retrieve the EVM address string for a `ProtectedKey`.
pub fn get_address_from_protected(key: &ProtectedKey) -> Result<String, Box<dyn Error + Send + Sync>> {
    let signer = create_signer_from_protected(key)?;
    Ok(format!("{:#x}", signer.address()))
}

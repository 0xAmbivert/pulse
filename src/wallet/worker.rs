use crate::crypto::{create_signer_from_protected, ProtectedKey};
use crate::wallet::nonce::NonceManager;
use alloy::consensus::{SignableTransaction, TxEip1559};
use alloy::eips::eip2718::Encodable2718;
use alloy::primitives::{Address, Bytes, TxKind, U256};
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::SignerSync;
use std::str::FromStr;
use std::sync::Arc;

pub struct WalletWorker {
    pub address: String,
    pub nonce_mgr: Arc<NonceManager>,
    /// The memory-pinned key.
    ///
    /// Retained so the key page stays `mlock`ed for the whole session and is scrubbed on
    /// drop, but it is no longer used to re-derive a signer per signature — see `signer`.
    pub key: ProtectedKey,
    /// The only long-lived signing capability.
    ///
    /// `SigningKey` implements `ZeroizeOnDrop`, so the secret material is scrubbed when this
    /// worker drops. It is still an ordinary (unlocked) heap allocation, which is an
    /// unavoidable consequence of using a signing library that owns its own key storage;
    /// the important property is that exactly one such copy exists for the session and it is
    /// never re-created per transaction.
    signer: PrivateKeySigner,
}

impl WalletWorker {
    pub fn new(key: ProtectedKey, initial_nonce: u64) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        // Derive the signer exactly once.
        let signer = create_signer_from_protected(&key)?;
        let addr = signer.address();
        let addr_str = format!("{:#x}", addr);
        let nonce_mgr = Arc::new(NonceManager::new(&addr_str, initial_nonce));

        Ok(Self {
            address: addr_str,
            nonce_mgr,
            key,
            signer,
        })
    }

    /// Fast-signs an EIP-1559 transaction locally in <1µs using the memory-pinned key.
    /// Returns the raw hex-encoded signed transaction ready for immediate parallel broadcasting.
    #[allow(clippy::too_many_arguments)]
    pub fn build_and_sign_eip1559(
        &self,
        chain_id: u64,
        to_address: &str,
        calldata: &[u8],
        value_wei: u128,
        gas_limit: u64,
        max_fee_wei: u128,
        max_priority_fee_wei: u128,
        explicit_nonce: Option<u64>,
    ) -> Result<(String, u64), Box<dyn std::error::Error + Send + Sync>> {
        let target_addr = Address::from_str(to_address)?;
        let reserved = explicit_nonce.is_none();
        let nonce = explicit_nonce.unwrap_or_else(|| self.nonce_mgr.get_and_increment());

        let tx = TxEip1559 {
            chain_id,
            nonce,
            gas_limit,
            max_fee_per_gas: max_fee_wei,
            max_priority_fee_per_gas: max_priority_fee_wei,
            to: TxKind::Call(target_addr),
            value: U256::from(value_wei),
            access_list: Default::default(),
            input: Bytes::copy_from_slice(calldata),
        };

        // Compute EIP-1559 signature hash
        let sighash = tx.signature_hash();
        // Sign with the session signer: no per-transaction key copy into unlocked memory.
        let signed = self.signer.sign_hash_sync(&sighash).map_err(Into::into);
        let signature = match signed {
            Ok(sig) => sig,
            Err(e) => {
                // Return the reserved nonce so a sign failure cannot leave a gap.
                if reserved {
                    self.nonce_mgr.release(nonce);
                }
                return Err(e);
            }
        };
        let signed_tx = tx.into_signed(signature);

        // Encode into standard EIP-2718 typed transaction payload
        let mut encoded = Vec::with_capacity(256);
        signed_tx.encode_2718(&mut encoded);

        let raw_hex = format!("0x{}", hex::encode(encoded));
        Ok((raw_hex, nonce))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_KEY: &str = "4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";

    fn worker() -> WalletWorker {
        WalletWorker::new(ProtectedKey::from_hex(TEST_KEY).expect("key"), 0).expect("worker")
    }

    #[test]
    fn signed_transactions_have_stable_sender_and_increment_nonce() {
        let w = worker();
        let to = "0x2222222222222222222222222222222222222222";

        let (first, nonce_a) = w
            .build_and_sign_eip1559(1, to, &[], 0, 21_000, 20_000_000_000, 1_000_000_000, None)
            .expect("sign 1");
        let (second, nonce_b) = w
            .build_and_sign_eip1559(1, to, &[], 0, 21_000, 20_000_000_000, 1_000_000_000, None)
            .expect("sign 2");

        assert_eq!(nonce_a, 0);
        assert_eq!(nonce_b, 1, "reserved nonces must advance");
        assert_ne!(first, second, "different nonces must produce different bytes");
        assert!(first.starts_with("0x02"), "expected EIP-1559 typed envelope");
    }

    #[test]
    fn explicit_nonce_does_not_consume_the_counter() {
        let w = worker();
        let to = "0x2222222222222222222222222222222222222222";

        let (_, nonce) = w
            .build_and_sign_eip1559(1, to, &[], 0, 21_000, 20_000_000_000, 1_000_000_000, Some(7))
            .expect("sign");
        assert_eq!(nonce, 7);
        assert_eq!(w.nonce_mgr.current(), 0, "explicit nonce must not reserve");
    }

    #[test]
    fn signer_is_derived_from_the_configured_address() {
        // Guards against the regression where a signer was re-derived per signature from a
        // source that could diverge from the worker's advertised address.
        let w = worker();
        assert_eq!(format!("{:#x}", w.signer.address()), w.address);
    }
}

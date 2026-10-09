use crate::crypto::{create_signer_from_protected, ProtectedKey};
use crate::wallet::nonce::NonceManager;
use alloy::consensus::{SignableTransaction, TxEip1559};
use alloy::eips::eip2718::Encodable2718;
use alloy::primitives::{Address, Bytes, TxKind, U256};
use alloy::signers::SignerSync;
use std::str::FromStr;
use std::sync::Arc;

use alloy::signers::local::PrivateKeySigner;

pub struct WalletWorker {
    pub address: String,
    pub nonce_mgr: Arc<NonceManager>,
    pub key: ProtectedKey,
    pub signer: PrivateKeySigner,
}

impl WalletWorker {
    pub fn new(key: ProtectedKey, initial_nonce: u64) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
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
        let signature = self.signer.sign_hash_sync(&sighash)?;
        let signed_tx = tx.into_signed(signature);

        // Encode into standard EIP-2718 typed transaction payload
        let mut encoded = Vec::with_capacity(256);
        signed_tx.encode_2718(&mut encoded);

        let raw_hex = format!("0x{}", hex::encode(encoded));
        Ok((raw_hex, nonce))
    }
}

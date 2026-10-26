use std::sync::Arc;
use crate::config::AppConfig;
use crate::gas::GasEngine;
use crate::wallet::WalletWorker;

#[derive(Clone)]
pub struct PreparedSnipe {
    pub worker: Arc<WalletWorker>,
    pub raw_tx: String,
    pub tx_hash: String,
    pub nonce: u64,
    pub max_fee_wei: u128,
    pub priority_fee_wei: u128,
    pub calldata: Vec<u8>,
    pub value_wei: u128,
}

pub fn prepare_snipes(
    workers: &[Arc<WalletWorker>],
    config: &AppConfig,
    target_contract: &str,
    current_base_fee_wei: u128,
    prev_base_fee_wei: Option<u128>,
    gas_engine: &GasEngine,
) -> Result<Vec<PreparedSnipe>, Box<dyn std::error::Error + Send + Sync>> {
    let mut snipes = Vec::with_capacity(workers.len());
    let (max_fee_wei, max_prio_wei) = gas_engine.calculate_dynamic_fees_with_acceleration(
        current_base_fee_wei,
        prev_base_fee_wei,
        1,
        None,
    )?;
    let value = crate::eligibility::seadrop_calldata::total_dispatch_value_wei(
        &config.drop,
        config.drop.mint_quantity,
    )?;
    let destination = config.drop.get_dispatch_destination(target_contract);

    for worker in workers {
        let calldata = crate::eligibility::seadrop_calldata::encode_dispatch_calldata(
            &config.drop,
            &worker.address,
            config.drop.mint_quantity,
        )?;
        let nonce = worker.nonce_mgr.current();
        let (raw_tx, n) = worker.build_and_sign_eip1559(
            config.chain.chain_id,
            destination,
            &calldata,
            value,
            config.gas.gas_limit,
            max_fee_wei,
            max_prio_wei,
            Some(nonce),
        )?;
        let tx_bytes = hex::decode(raw_tx.trim_start_matches("0x"))
            .map_err(|e| format!("Signed transaction is not valid hex: {e}"))?;
        let tx_hash = format!("{:#x}", alloy::primitives::keccak256(&tx_bytes));
        snipes.push(PreparedSnipe {
            worker: worker.clone(),
            raw_tx,
            tx_hash,
            nonce: n,
            max_fee_wei,
            priority_fee_wei: max_prio_wei,
            calldata,
            value_wei: value,
        });
    }
    Ok(snipes)
}

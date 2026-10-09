use std::sync::Arc;
use tokio::sync::mpsc;
use crate::alerts::AlertDispatcher;
use crate::gas::{wei_to_gwei, GasEngine};
use crate::network::{MevBuilderClient, RpcRacer};
use crate::wallet::WalletWorker;

pub struct SubmittedTx {
    pub worker: Arc<WalletWorker>,
    pub tx_hash: String,
    pub nonce: u64,
    pub max_fee_wei: u128,
    pub priority_fee_wei: u128,
    pub calldata: Vec<u8>,
    pub value_wei: u128,
    pub last_bump: std::time::Instant,
    pub bump_count: u32,
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_tx_monitor(
    mut tx: SubmittedTx,
    rpc_racer: Arc<RpcRacer>,
    mev_client: Option<Arc<MevBuilderClient>>,
    alerts: Arc<AlertDispatcher>,
    gas_engine: Arc<GasEngine>,
    chain_id: u64,
    target_contract: String,
    auto_speedup: bool,
    speedup_threshold_ms: u64,
    gas_limit: u64,
    ui_log_tx: mpsc::Sender<String>,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
        loop {
            interval.tick().await;

            if let Some(receipt) = rpc_racer.poll_receipt(&tx.tx_hash).await {
                let status_ok = receipt
                    .get("status")
                    .and_then(|s| s.as_str())
                    .is_some_and(|s| s == "0x1" || s == "1");
                let block_num = receipt
                    .get("blockNumber")
                    .and_then(|b| b.as_str())
                    .unwrap_or("unknown");
                let gas_used = receipt
                    .get("gasUsed")
                    .and_then(|g| g.as_str())
                    .unwrap_or("unknown");

                if status_ok {
                    let _ = ui_log_tx
                        .send(format!("🎉 Tx Confirmed! {} (Block: {})", tx.tx_hash, block_num))
                        .await;
                    let alerts_c = alerts.clone();
                    let msg = format!(
                        "Wallet: {}\nTx: {}\nBlock: {}\nGas: {}",
                        tx.worker.address, tx.tx_hash, block_num, gas_used
                    );
                    tokio::spawn(async move {
                        alerts_c.dispatch_alert("🎯 Mint Confirmed", &msg, true).await;
                    });
                } else {
                    let _ = ui_log_tx
                        .send(format!("❌ Tx Reverted on-chain: {}", tx.tx_hash))
                        .await;
                    let alerts_c = alerts.clone();
                    let msg = format!(
                        "Wallet: {}\nTx: {}\nReverted in block: {}",
                        tx.worker.address, tx.tx_hash, block_num
                    );
                    tokio::spawn(async move {
                        alerts_c.dispatch_alert("⚠️ Mint Reverted", &msg, false).await;
                    });
                }
                break;
            } else if auto_speedup
                && tx.last_bump.elapsed().as_millis() >= speedup_threshold_ms as u128
            {
                match gas_engine.calculate_speedup_fees(tx.max_fee_wei, tx.priority_fee_wei) {
                    Ok((new_max, new_prio)) => {
                        match tx.worker.build_and_sign_eip1559(
                            chain_id,
                            &target_contract,
                            &tx.calldata,
                            tx.value_wei,
                            gas_limit,
                            new_max,
                            new_prio,
                            Some(tx.nonce),
                        ) {
                            Ok((new_raw, _)) => {
                                let new_bytes = hex::decode(new_raw.trim_start_matches("0x"))
                                    .unwrap_or_default();
                                let new_hash =
                                    format!("{:#x}", alloy::primitives::keccak256(&new_bytes));
                                tx.tx_hash = new_hash.clone();
                                tx.max_fee_wei = new_max;
                                tx.priority_fee_wei = new_prio;
                                tx.last_bump = std::time::Instant::now();
                                tx.bump_count += 1;

                                let _ = ui_log_tx
                                    .send(format!(
                                        "⚡ Speedup #{} for {} (Max: {:.2} Gwei)",
                                        tx.bump_count,
                                        tx.worker.address,
                                        wei_to_gwei(new_max)
                                    ))
                                    .await;

                                let alerts_c = alerts.clone();
                                let addr = tx.worker.address.clone();
                                let cnt = tx.bump_count;
                                tokio::spawn(async move {
                                    alerts_c
                                        .dispatch_alert(
                                            "⚡ Speedup Bumped",
                                            &format!(
                                                "Wallet: {}\nTx: {}\nBump: #{}\nMax: {:.2} Gwei",
                                                addr,
                                                new_hash,
                                                cnt,
                                                wei_to_gwei(new_max)
                                            ),
                                            true,
                                        )
                                        .await;
                                });

                                if let Some(mev) = &mev_client {
                                    let mev_c = mev.clone();
                                    let raw_c = new_raw.clone();
                                    let alerts_c = alerts.clone();
                                    tokio::spawn(async move {
                                        let outcomes =
                                            mev_c.send_private_transaction(&raw_c).await;
                                        if !outcomes.iter().any(|o| o.success) {
                                            alerts_c
                                                .dispatch_alert(
                                                    "⚠️ All Builders Rejected Speedup",
                                                    &format!("{:?}", outcomes),
                                                    false,
                                                )
                                                .await;
                                        }
                                    });
                                } else {
                                    let racer_c = rpc_racer.clone();
                                    let raw_c = new_raw.clone();
                                    tokio::spawn(async move {
                                        let _ = racer_c.race_broadcast_raw_tx(&raw_c).await;
                                    });
                                }
                            }
                            Err(e) => {
                                let _ = ui_log_tx
                                    .send(format!("⚠️ Speedup sign error: {}", e))
                                    .await;
                            }
                        }
                    }
                    Err(e) => {
                        let _ = ui_log_tx
                            .send(format!("⚠️ Speedup ceiling: {}", e))
                            .await;
                        tx.last_bump = std::time::Instant::now();
                    }
                }
            }
        }
    });
}

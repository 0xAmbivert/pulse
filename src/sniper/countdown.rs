use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tracing::info;

#[derive(Debug, Clone)]
pub enum SnipeTrigger {
    CountdownReached { target_unix: u64 },
    BlockReached { target_block: u64 },
    MempoolDetected { owner_tx_hash: String, method: String },
    StateFlipDetected { new_state: bool },
    BaseFeeUpdated { base_fee_wei: u128 },
}

pub struct CountdownSniper {
    target_timestamp: u64,
    lead_time_ms: u64, // Lead time before drop to pre-fire/burst
}

impl CountdownSniper {
    pub fn new(target_timestamp: u64, lead_time_ms: u64) -> Self {
        Self {
            target_timestamp,
            lead_time_ms,
        }
    }

    /// Runs countdown loop until target timestamp (minus lead_time_ms), then triggers the dispatch channel.
    pub async fn wait_for_target(&self, tx: mpsc::Sender<SnipeTrigger>) {
        loop {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_millis() as u64;

            let target_ms = self.target_timestamp.saturating_mul(1000);
            let fire_ms = target_ms.saturating_sub(self.lead_time_ms);

            if now >= fire_ms {
                info!("Countdown reached target {}! Disagreeing lead time {}ms. Triggering burst!", self.target_timestamp, self.lead_time_ms);
                let _ = tx.send(SnipeTrigger::CountdownReached { target_unix: self.target_timestamp }).await;
                break;
            }

            let diff_ms = fire_ms - now;
            if diff_ms > 1000 {
                tokio::time::sleep(Duration::from_millis(500)).await;
            } else if diff_ms > 50 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            } else {
                // Micro-spin for the last 50 milliseconds
                tokio::task::yield_now().await;
            }
        }
    }
}

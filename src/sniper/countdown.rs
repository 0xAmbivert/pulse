use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tracing::info;

#[derive(Debug, Clone)]
pub enum SnipeTrigger {
    CountdownReached { target_unix: u64 },
    MempoolDetected { owner_tx_hash: String, method: String },
    StateFlipDetected { new_state: bool },
    BaseFeeUpdated { base_fee_wei: u128 },
}

pub struct CountdownSniper {
    target_timestamp: u64,
    lead_time_ms: u64, // Lead time before drop to pre-fire/burst
    clock_offset_ms: i64, // Calibration offset against network block time / NTP
}

impl CountdownSniper {
    pub fn new(target_timestamp: u64, lead_time_ms: u64) -> Self {
        Self::with_offset(target_timestamp, lead_time_ms, 0)
    }

    pub fn with_offset(target_timestamp: u64, lead_time_ms: u64, clock_offset_ms: i64) -> Self {
        Self {
            target_timestamp,
            lead_time_ms,
            clock_offset_ms,
        }
    }

    pub fn set_clock_offset_ms(&mut self, clock_offset_ms: i64) {
        self.clock_offset_ms = clock_offset_ms;
    }

    pub fn clock_offset_ms(&self) -> i64 {
        self.clock_offset_ms
    }

    /// Runs countdown loop until target timestamp (minus lead_time_ms and offset),
    /// then triggers the dispatch channel.
    pub async fn wait_for_target(&self, tx: mpsc::Sender<SnipeTrigger>) {
        let target_ms = self.target_timestamp.saturating_mul(1000);
        let effective_lead = self.lead_time_ms as i64 + self.clock_offset_ms;
        let fire_ms = if effective_lead >= 0 {
            target_ms.saturating_sub(effective_lead as u64)
        } else {
            target_ms.saturating_add((-effective_lead) as u64)
        };

        loop {
            let raw_now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_millis() as u64;

            if raw_now >= fire_ms {
                info!("Countdown reached target {}! Lead time {}ms. Triggering burst!", self.target_timestamp, self.lead_time_ms);
                let _ = tx.send(SnipeTrigger::CountdownReached { target_unix: self.target_timestamp }).await;
                break;
            }

            let diff_ms = fire_ms - raw_now;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_countdown_sniper_instant_trigger() {
        let (tx, mut rx) = mpsc::channel(1);
        let past_timestamp = 100;
        let sniper = CountdownSniper::new(past_timestamp, 0);
        sniper.wait_for_target(tx).await;
        let received = rx.recv().await.unwrap();
        assert!(matches!(received, SnipeTrigger::CountdownReached { target_unix } if target_unix == past_timestamp));
    }

    #[tokio::test]
    async fn test_countdown_sniper_with_offset() {
        let (tx, mut rx) = mpsc::channel(1);
        let now_sec = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let target_ts = now_sec + 1;
        let sniper = CountdownSniper::with_offset(target_ts, 2000, 0);
        sniper.wait_for_target(tx).await;
        let received = rx.recv().await.unwrap();
        assert!(matches!(received, SnipeTrigger::CountdownReached { target_unix } if target_unix == target_ts));
    }
}

use reqwest::Client;
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub struct NonceManager {
    address: String,
    current_nonce: AtomicU64,
}

impl NonceManager {
    pub fn new(address: &str, initial_nonce: u64) -> Self {
        Self {
            address: address.to_string(),
            current_nonce: AtomicU64::new(initial_nonce),
        }
    }

    /// Returns the current nonce and increments it atomically for immediate subsequent use.
    pub fn get_and_increment(&self) -> u64 {
        self.current_nonce.fetch_add(1, Ordering::SeqCst)
    }

    /// Reads the current cached nonce without incrementing.
    pub fn current(&self) -> u64 {
        self.current_nonce.load(Ordering::SeqCst)
    }

    /// Sets the nonce explicitly.
    ///
    /// This overwrites unconditionally, so it is only safe for deliberate resets (startup
    /// and explicit `resync_from_rpc`). Background observers must use [`Self::advance`].
    pub fn set_nonce(&self, nonce: u64) {
        self.current_nonce.store(nonce, Ordering::SeqCst);
    }

    /// Moves the nonce forward to `nonce` if it is ahead, never backwards.
    ///
    /// Chain observers (the receipt monitor, nonce-gap detection) learn on-chain state
    /// asynchronously and can be overtaken. A plain store from a stale observation would
    /// rewind the counter and make the next transaction reuse a consumed nonce, which the
    /// node rejects with `nonce too low`. `fetch_max` keeps the counter monotonic under
    /// concurrent writers.
    ///
    /// Returns the value in effect after the update.
    pub fn advance(&self, nonce: u64) -> u64 {
        // `fetch_max` returns the *previous* value, so reload to report the current one.
        let previous = self.current_nonce.fetch_max(nonce, Ordering::SeqCst);
        previous.max(nonce)
    }

    /// Rolls back a reserved nonce only if no later nonce was reserved since.
    pub fn release(&self, nonce: u64) {
        let _ = self.current_nonce.compare_exchange(nonce + 1, nonce, Ordering::SeqCst, Ordering::SeqCst);
    }

    /// Resynchronizes the nonce by querying `eth_getTransactionCount` with tag "pending" from RPC.
    pub async fn resync_from_rpc(&self, rpc_url: &str) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        static CLIENT: std::sync::OnceLock<Client> = std::sync::OnceLock::new();
        let client = match CLIENT.get() {
            Some(c) => c,
            None => {
                let built = Client::builder().timeout(Duration::from_millis(3000)).build()?;
                CLIENT.get_or_init(|| built)
            }
        };

        let payload = json!({
            "jsonrpc": "2.0",
            "method": "eth_getTransactionCount",
            "params": [self.address, "pending"],
            "id": 1
        });

        let resp = client.post(rpc_url).json(&payload).send().await?.error_for_status()?;
        let body: serde_json::Value = resp.json().await?;

        if let Some(err) = body.get("error") {
            return Err(format!("RPC error fetching nonce: {err}").into());
        }
        let result_hex = body
            .get("result")
            .and_then(|r| r.as_str())
            .ok_or("Failed to parse nonce from RPC response")?;
        let nonce = u64::from_str_radix(result_hex.trim_start_matches("0x"), 16)?;
        self.set_nonce(nonce);
        Ok(nonce)
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nonce_manager_atomic_increment() {
        let mgr = NonceManager::new("0x1234567890123456789012345678901234567890", 5);
        assert_eq!(mgr.current(), 5);
        assert_eq!(mgr.get_and_increment(), 5);
        assert_eq!(mgr.current(), 6);
        mgr.set_nonce(10);
        assert_eq!(mgr.current(), 10);
    }

    #[test]
    fn test_advance_never_rewinds() {
        let mgr = NonceManager::new("0x1234567890123456789012345678901234567890", 10);
        // A stale observation from a lagging endpoint must not rewind the counter.
        assert_eq!(mgr.advance(7), 10);
        assert_eq!(mgr.current(), 10);
        // A newer observation moves it forward.
        assert_eq!(mgr.advance(12), 12);
        assert_eq!(mgr.current(), 12);
    }

    #[test]
    fn test_advance_is_monotonic_across_reservation() {
        let mgr = NonceManager::new("0x1234567890123456789012345678901234567890", 0);
        assert_eq!(mgr.get_and_increment(), 0);
        assert_eq!(mgr.get_and_increment(), 1);
        // Chain confirms nonce 0 only; reserving 1 locally must survive the sync.
        assert_eq!(mgr.advance(1), 2);
        assert_eq!(mgr.current(), 2);
    }
}

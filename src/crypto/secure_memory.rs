use std::fmt;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// A memory-protected container for sensitive cryptographic secrets (e.g. 32-byte private keys).
/// - Automatically calls `libc::mlock` upon allocation to prevent OS paging/swap leaks.
/// - Implements `ZeroizeOnDrop` to overwrite memory with zeros immediately when dropped.
/// - Does not implement `Clone` or `Debug` (prevents accidental duplication or logging).
#[derive(Zeroize)]
pub struct ProtectedKey {
    bytes: Box<[u8; 32]>,
    #[zeroize(skip)]
    is_locked: bool,
}

impl ProtectedKey {
    /// Creates a new `ProtectedKey` from a 32-byte slice, pinning the memory in RAM.
    pub fn new(key_bytes: [u8; 32]) -> Self {
        let mut key = Self {
            bytes: Box::new(key_bytes),
            is_locked: false,
        };
        key.lock_memory();
        key
    }

    /// Attempts to parse a 64-character hex string (with or without 0x prefix).
    pub fn from_hex(hex_str: &str) -> Result<Self, String> {
        let clean_hex = hex_str.trim().trim_start_matches("0x");
        if clean_hex.len() != 64 {
            return Err(format!(
                "Invalid private key length: expected 64 hex characters, got {}",
                clean_hex.len()
            ));
        }

        let mut bytes = [0u8; 32];
        hex::decode_to_slice(clean_hex, &mut bytes)
            .map_err(|e| format!("Invalid hex encoding for private key: {e}"))?;

        Ok(Self::new(bytes))
    }

    /// Locks the underlying memory buffer using `libc::mlock` to ensure the OS never writes it to swap.
    fn lock_memory(&mut self) {
        #[cfg(target_os = "linux")]
        unsafe {
            let ptr = self.bytes.as_ptr() as *const libc::c_void;
            let len = self.bytes.len();
            if libc::mlock(ptr, len) == 0 {
                self.is_locked = true;
            } else {
                panic!("FATAL: Failed to mlock private key memory (RLIMIT_MEMLOCK exceeded?). OS swap leak possible. Aborting.");
            }
        }
    }

    /// Safely provides temporary read access to the private key bytes.
    #[inline(always)]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.bytes
    }

    /// Returns whether the memory page is successfully pinned via `mlock`.
    pub fn is_memory_locked(&self) -> bool {
        self.is_locked
    }
}

impl Drop for ProtectedKey {
    fn drop(&mut self) {
        // Unlock memory before deallocation if it was locked
        if self.is_locked {
            #[cfg(target_os = "linux")]
            unsafe {
                let ptr = self.bytes.as_ptr() as *const libc::c_void;
                let len = self.bytes.len();
                let _ = libc::munlock(ptr, len);
            }
        }
        // Zeroize is explicitly called
        self.bytes.as_mut().zeroize();
    }
}

impl fmt::Debug for ProtectedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ProtectedKey {{ [REDACTED - 32 bytes; mlock: {}] }}",
            self.is_locked
        )
    }
}

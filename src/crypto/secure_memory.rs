use std::fmt;
use zeroize::Zeroize;

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
    /// Allocates an empty 32-byte pinned heap buffer.
    pub fn empty() -> Self {
        let mut key = Self {
            bytes: Box::new([0u8; 32]),
            is_locked: false,
        };
        key.lock_memory();
        key
    }

    /// Mutably borrow the underlying pinned buffer to securely copy data directly into it without stack copies.
    pub fn as_mut_bytes(&mut self) -> &mut [u8; 32] {
        self.bytes.as_mut()
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

        let mut key = Self::empty();
        hex::decode_to_slice(clean_hex, key.as_mut_bytes())
            .map_err(|e| format!("Invalid hex encoding for private key: {e}"))?;

        Ok(key)
    }

    /// Locks the underlying memory buffer using `libc::mlock` to ensure the OS never writes it to swap.
    fn lock_memory(&mut self) {
        #[cfg(unix)]
        unsafe {
            let ptr = self.bytes.as_ptr() as *const libc::c_void;
            let len = self.bytes.len();
            if libc::mlock(ptr, len) == 0 {
                self.is_locked = true;
            } else {
                eprintln!("WARNING: Failed to mlock private key memory (RLIMIT_MEMLOCK exceeded?). Key is isolated to heap but may be swapped.");
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
        // Zeroize MUST happen before unlocking memory
        self.bytes.as_mut().zeroize();
        // Unlock memory before deallocation if it was locked
        if self.is_locked {
            #[cfg(unix)]
            unsafe {
                let ptr = self.bytes.as_ptr() as *const libc::c_void;
                let len = self.bytes.len();
                let _ = libc::munlock(ptr, len);
            }
        }
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

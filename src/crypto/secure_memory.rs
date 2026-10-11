use std::fmt;
use std::ptr;
use zeroize::Zeroizing;

const KEY_LEN: usize = 32;
/// Set to "1" to allow running with an unlocked (swappable) key page when mlock is not permitted.
const ALLOW_UNLOCKED_ENV: &str = "PULSE_ALLOW_UNLOCKED_KEY";

/// Disables core dumps and ptrace/`/proc/<pid>/mem` access for this process.
/// Must be called once at startup, before any secret is loaded.
pub fn harden_process() {
    unsafe {
        let zero_limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        let _ = libc::setrlimit(libc::RLIMIT_CORE, &zero_limit);
        #[cfg(target_os = "linux")]
        {
            let _ = libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
        }
    }
}

/// A memory-protected container for sensitive cryptographic secrets (e.g. 32-byte private keys).
/// - Lives on a dedicated anonymous page (`mmap`), so `munlock` never touches unrelated secrets.
/// - Pinned with `mlock` and excluded from core dumps (`MADV_DONTDUMP`) on Linux.
/// - Zeroized before the page is unlocked and unmapped.
/// - Fails closed: if the page cannot be locked, construction returns an error unless
///   `PULSE_ALLOW_UNLOCKED_KEY=1` is explicitly set.
/// - Does not implement `Clone` or `Debug` (prevents accidental duplication or logging).
pub struct ProtectedKey {
    page: *mut libc::c_void,
    page_len: usize,
    is_locked: bool,
}

// SAFETY: the page is uniquely owned by this value and is never aliased.
unsafe impl Send for ProtectedKey {}
// SAFETY: only shared references (&[u8; 32]) are handed out; no interior mutability.
unsafe impl Sync for ProtectedKey {}

impl ProtectedKey {
    /// Allocates a zeroed, dedicated page and pins it in RAM.
    pub fn empty() -> Result<Self, String> {
        let page_len = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(4096) as usize;
        let page = unsafe {
            libc::mmap(
                ptr::null_mut(),
                page_len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if page == libc::MAP_FAILED {
            return Err("Failed to allocate memory for private key".to_string());
        }

        // From here on, Drop cleans up the page on every error path.
        let mut key = Self {
            page,
            page_len,
            is_locked: false,
        };
        key.lock_memory()?;
        Ok(key)
    }

    /// Mutably borrow the key buffer to copy data directly into it.
    pub fn as_mut_bytes(&mut self) -> &mut [u8; KEY_LEN] {
        unsafe { &mut *(self.page as *mut [u8; KEY_LEN]) }
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

        let mut decoded = Zeroizing::new([0u8; KEY_LEN]);
        hex::decode_to_slice(clean_hex, decoded.as_mut())
            .map_err(|e| format!("Invalid hex encoding for private key: {e}"))?;

        let mut key = Self::empty()?;
        key.as_mut_bytes().copy_from_slice(decoded.as_ref());
        Ok(key)
    }

    /// Locks the key page and marks it as excluded from core dumps.
    fn lock_memory(&mut self) -> Result<(), String> {
        #[cfg(target_os = "linux")]
        unsafe {
            // Best effort: keep the page out of core dumps.
            let _ = libc::madvise(self.page, self.page_len, libc::MADV_DONTDUMP);
        }

        if unsafe { libc::mlock(self.page, self.page_len) } == 0 {
            self.is_locked = true;
            return Ok(());
        }

        if std::env::var(ALLOW_UNLOCKED_ENV).as_deref() == Ok("1") {
            eprintln!("WARNING: mlock failed; continuing because {ALLOW_UNLOCKED_ENV}=1. Key may be swapped to disk.");
            Ok(())
        } else {
            Err(format!(
                "Failed to mlock private key memory (RLIMIT_MEMLOCK too low?). Refusing to continue. Set {ALLOW_UNLOCKED_ENV}=1 to override."
            ))
        }
    }

    /// Safely provides temporary read access to the private key bytes.
    #[inline(always)]
    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        unsafe { &*(self.page as *const [u8; KEY_LEN]) }
    }

    /// Returns whether the memory page is successfully pinned via `mlock`.
    pub fn is_memory_locked(&self) -> bool {
        self.is_locked
    }
}

impl Drop for ProtectedKey {
    fn drop(&mut self) {
        if self.page.is_null() || self.page == libc::MAP_FAILED {
            return;
        }
        unsafe {
            // Zeroize via volatile writes so the compiler cannot elide them.
            let bytes = self.as_mut_bytes();
            for b in bytes.iter_mut() {
                ptr::write_volatile(b, 0);
            }
            std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
            if self.is_locked {
                let _ = libc::munlock(self.page, self.page_len);
            }
            let _ = libc::munmap(self.page, self.page_len);
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

pub mod webhooks;

use std::path::PathBuf;
pub use webhooks::AlertDispatcher;

/// Opens the diagnostic log at `$XDG_STATE_HOME/pulse/pulse.log` (falling back to
/// `~/.local/state/pulse/pulse.log`, then the temp dir) with owner-only permissions.
///
/// Returns `None` if no writable location exists; logging is best-effort.
pub fn open_log_file() -> Option<std::fs::File> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(std::env::temp_dir)
        .join("pulse");

    if let Err(e) = std::fs::create_dir_all(&base) {
        eprintln!("warning: cannot create log directory {}: {e}", base.display());
        return None;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(base.join("pulse.log"))
            .ok()
    }
    #[cfg(not(unix))]
    {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(base.join("pulse.log"))
            .ok()
    }
}

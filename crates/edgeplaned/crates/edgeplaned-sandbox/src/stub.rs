use crate::error::Result;
use std::sync::OnceLock;

static WARNED: OnceLock<()> = OnceLock::new();

pub fn apply_sandbox(_allowed: &[String]) -> Result<()> {
    WARNED.get_or_init(|| {
        eprintln!("[edgeplaned] WARNING: OS-level sandbox is not available on this platform.");
        eprintln!("[edgeplaned] All capabilities run without kernel-level isolation.");
    });
    Ok(())
}

pub fn sandbox_enforced() -> bool {
    false
}

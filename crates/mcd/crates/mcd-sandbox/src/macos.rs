use crate::error::Result;
use crate::types::SideEffectClass;

static SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// True if the macOS `sandbox-exec` binary is present on this system.
pub fn sandbox_exec_available() -> bool {
    std::path::Path::new(SANDBOX_EXEC).exists()
}

pub fn sandbox_enforced() -> bool {
    sandbox_exec_available()
}

/// No-op on macOS — sandboxing is applied per-subprocess via sandbox-exec, not to the
/// current process.
pub fn apply_sandbox(_allowed: &[String]) -> Result<()> {
    Ok(())
}

/// Returns an inline SBPL profile string for the given side effect class.
///
/// Returns `None` for classes that should not be sandbox-wrapped (e.g. Additive/Mutating
/// capabilities need write access and the policy layer is the enforcement boundary there).
pub fn profile_for(class: &SideEffectClass) -> Option<&'static str> {
    match class {
        SideEffectClass::None | SideEffectClass::ReadOnly => Some(READ_ONLY_PROFILE),
        _ => None,
    }
}

/// SBPL profile for read-only and side-effect-free capability execution.
///
/// Grants: process execution, file read, network, IPC/mach (needed by most CLIs).
/// Denies: all file writes — the key invariant for readOnly capabilities.
const READ_ONLY_PROFILE: &str = "\
(version 1)\
(deny default)\
(allow process*)\
(allow file-read*)\
(allow network*)\
(allow sysctl-read)\
(allow mach*)\
(allow ipc*)\
(allow signal)\
(deny file-write*)\
";

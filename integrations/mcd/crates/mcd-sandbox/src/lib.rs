pub mod error;
pub mod types;
pub mod jail;

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod stub;

#[cfg(target_os = "linux")]
pub mod seccomp;

pub use error::{Result, SandboxError};
pub use types::{CgroupLimits, FsPolicy, NetworkPolicy, SideEffectClass};
pub use jail::{JailConfig, discover_lib_deps, enter_jail, resolve_and_hash_binary, verify_binary_hash};

/// Apply OS-level sandbox restrictions to the current process.
///
/// On Linux: installs a Landlock execute ruleset restricting to `allowed_executables`.
/// On macOS: no-op (sandboxing is applied per-subprocess via sandbox-exec).
/// On other platforms: no-op with a warning.
pub fn apply_sandbox(allowed_executables: &[impl AsRef<str>]) -> Result<()> {
    let paths: Vec<String> = allowed_executables.iter().map(|s| s.as_ref().to_string()).collect();
    #[cfg(target_os = "linux")]
    return linux::apply_sandbox(&paths);
    #[cfg(target_os = "macos")]
    return macos::apply_sandbox(&paths);
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    return stub::apply_sandbox(&paths);
}

/// Returns true if OS-level sandboxing is active/enforceable on this platform.
pub fn sandbox_enforced() -> bool {
    #[cfg(target_os = "linux")]
    return linux::sandbox_enforced();
    #[cfg(target_os = "macos")]
    return macos::sandbox_enforced();
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    return stub::sandbox_enforced();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_flag() {
        let _ = sandbox_enforced();
    }

    #[test]
    fn test_empty_allowlist_noop() {
        let empty: Vec<String> = vec![];
        apply_sandbox(&empty).unwrap();
    }

    #[test]
    fn test_default_cgroup_limits() {
        let limits = CgroupLimits::default();
        assert_eq!(limits.memory_mib, 512);
        assert_eq!(limits.max_pids, 64);
        assert_eq!(limits.cpu_weight, 100);
    }

    #[test]
    fn test_default_fs_policy() {
        let policy = FsPolicy::default();
        assert!(policy.extra_ro_bind.is_empty());
        assert!(policy.extra_rw_bind.is_empty());
        assert!(!policy.share_host_tmp);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_apply_sandbox_ls() {
        // Should succeed or return an error — must not panic
        let result = apply_sandbox(&["/bin/ls"]);
        // Landlock may fail if the binary doesn't exist at that exact path or if
        // Landlock is not supported on this kernel; either way, no panic.
        let _ = result;
    }
}

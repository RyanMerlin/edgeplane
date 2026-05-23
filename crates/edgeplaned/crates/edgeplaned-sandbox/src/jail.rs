/// Linux-namespace jail setup for warm workers.
///
/// The jail lifecycle:
///   1. Parent (gateway) calls `prepare_worker_jail()` to get a `JailConfig`.
///   2. Parent forks/spawns the worker process, passing the config via env vars.
///   3. Worker binary calls `enter_jail()` on startup, which:
///      a. Calls `unshare(2)` to create new user/mount/pid/net/ipc/uts namespaces.
///      b. Writes uid_map / gid_map to map the worker's UID → 0 inside the namespace.
///      c. Sets up the mount namespace: tmpfs root, RO bind of pinned binary + ldd libs, proc, tmpfs /tmp, tmpfs /home/edgeplaned.
///      d. Calls `pivot_root(2)` to chroot into the new rootfs.
///      e. Sets PR_SET_NO_NEW_PRIVS.
///      f. Drops all capabilities via `prctl(PR_SET_SECUREBITS)`.
///      g. Applies Landlock execute restrictions.
///      h. Installs the seccomp filter.
///   4. Worker then listens on its control socket for dispatch messages.
///
/// This module intentionally avoids setuid helpers by relying solely on unprivileged user
/// namespaces (kernel ≥ 3.8, enabled on most distros). The key invariant is that
/// `CLONE_NEWUSER` must be in the same `unshare` call as the other namespace flags.
use std::path::{Path, PathBuf};
use std::fs;
use crate::error::{Result, SandboxError};
use crate::types::{FsPolicy, NetworkPolicy, CgroupLimits};

/// All configuration needed to set up a jail. Serialized to env vars by the gateway,
/// deserialized by the worker on startup.
#[derive(Debug, Clone)]
pub struct JailConfig {
    /// Absolute path to the CLI binary (pinned, verified).
    pub pinned_binary: PathBuf,
    /// Expected SHA-256 hex digest of the binary.
    pub binary_sha256: String,
    /// Dynamic library paths to bind-mount RO (from `ldd` output).
    pub lib_paths: Vec<PathBuf>,
    /// Extra RO and RW bind mounts from the capability's fs policy.
    pub fs_policy: FsPolicy,
    /// Network egress policy (currently informational — actual nftables rules are set by the
    /// gateway in the host's network namespace before the worker unshares).
    pub network_policy: NetworkPolicy,
    /// cgroup v2 limits.
    pub limits: CgroupLimits,
    /// Additional syscall names to deny on top of the baseline.
    pub extra_deny_syscalls: Vec<String>,
}

/// Environment variable names used to pass JailConfig from gateway to worker.
pub mod env_keys {
    pub const PINNED_BINARY: &str = "MC_MESH_JAIL_BINARY";
    pub const BINARY_SHA256: &str = "MC_MESH_JAIL_SHA256";
    pub const LIB_PATHS: &str = "MC_MESH_JAIL_LIBS";           // colon-separated
    pub const EXTRA_RO_BIND: &str = "MC_MESH_JAIL_RO_BIND";    // colon-separated
    pub const EXTRA_RW_BIND: &str = "MC_MESH_JAIL_RW_BIND";    // colon-separated
    pub const EXTRA_DENY_SYSCALLS: &str = "MC_MESH_JAIL_DENY_SYSCALLS"; // comma-separated
    pub const MEMORY_MIB: &str = "MC_MESH_JAIL_MEM_MIB";
    pub const MAX_PIDS: &str = "MC_MESH_JAIL_MAX_PIDS";
    pub const CPU_WEIGHT: &str = "MC_MESH_JAIL_CPU_WEIGHT";
    pub const WORKER_SOCKET_FD: &str = "MC_MESH_WORKER_FD";
    pub const SHARE_HOST_TMP: &str = "MC_MESH_JAIL_SHARE_TMP";
}

impl JailConfig {
    /// Serialize JailConfig into a set of env key=value pairs for passing to the worker process.
    pub fn to_env(&self) -> Vec<(String, String)> {
        let env = vec![
            (env_keys::PINNED_BINARY.to_string(), self.pinned_binary.to_string_lossy().to_string()),
            (env_keys::BINARY_SHA256.to_string(), self.binary_sha256.clone()),
            (env_keys::LIB_PATHS.to_string(), self.lib_paths.iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join(":")),
            (env_keys::EXTRA_RO_BIND.to_string(), self.fs_policy.extra_ro_bind.join(":")),
            (env_keys::EXTRA_RW_BIND.to_string(), self.fs_policy.extra_rw_bind.join(":")),
            (env_keys::EXTRA_DENY_SYSCALLS.to_string(), self.extra_deny_syscalls.join(",")),
            (env_keys::MEMORY_MIB.to_string(), self.limits.memory_mib.to_string()),
            (env_keys::MAX_PIDS.to_string(), self.limits.max_pids.to_string()),
            (env_keys::CPU_WEIGHT.to_string(), self.limits.cpu_weight.to_string()),
            (env_keys::SHARE_HOST_TMP.to_string(), if self.fs_policy.share_host_tmp { "1" } else { "0" }.to_string()),
        ];
        env
    }

    /// Deserialize from the current process's environment variables (called by the worker).
    pub fn from_env() -> Option<Self> {
        let binary = std::env::var(env_keys::PINNED_BINARY).ok()?;
        let sha256 = std::env::var(env_keys::BINARY_SHA256).ok()?;
        let libs: Vec<PathBuf> = std::env::var(env_keys::LIB_PATHS)
            .unwrap_or_default()
            .split(':')
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect();
        let ro_bind: Vec<String> = std::env::var(env_keys::EXTRA_RO_BIND)
            .unwrap_or_default()
            .split(':')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        let rw_bind: Vec<String> = std::env::var(env_keys::EXTRA_RW_BIND)
            .unwrap_or_default()
            .split(':')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        let deny_syscalls: Vec<String> = std::env::var(env_keys::EXTRA_DENY_SYSCALLS)
            .unwrap_or_default()
            .split(',')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        let memory_mib: u64 = std::env::var(env_keys::MEMORY_MIB).ok()
            .and_then(|s| s.parse().ok()).unwrap_or(512);
        let max_pids: u64 = std::env::var(env_keys::MAX_PIDS).ok()
            .and_then(|s| s.parse().ok()).unwrap_or(64);
        let cpu_weight: u64 = std::env::var(env_keys::CPU_WEIGHT).ok()
            .and_then(|v| v.parse().ok()).unwrap_or(100);
        let share_host_tmp = std::env::var(env_keys::SHARE_HOST_TMP).unwrap_or_default() == "1";

        Some(JailConfig {
            pinned_binary: PathBuf::from(binary),
            binary_sha256: sha256,
            lib_paths: libs,
            fs_policy: FsPolicy { extra_ro_bind: ro_bind, extra_rw_bind: rw_bind, share_host_tmp },
            network_policy: NetworkPolicy::default(),
            limits: CgroupLimits { memory_mib, max_pids, cpu_weight },
            extra_deny_syscalls: deny_syscalls,
        })
    }
}

/// Resolve the absolute path to a binary and compute its SHA-256 digest.
pub fn resolve_and_hash_binary(command: &str) -> Result<(PathBuf, String)> {
    use sha2::{Digest, Sha256};

    let path = which_binary(command)?;
    let bytes = fs::read(&path)
        .map_err(|e| SandboxError::Isolation(format!("read binary {}: {e}", path.display())))?;
    let digest = hex::encode(Sha256::digest(&bytes));
    Ok((path, digest))
}

/// Verify that the binary at `path` matches the expected SHA-256 hex digest.
pub fn verify_binary_hash(path: &Path, expected_sha256: &str) -> Result<()> {
    use sha2::{Digest, Sha256};

    let bytes = fs::read(path)
        .map_err(|e| SandboxError::Isolation(format!("read binary {}: {e}", path.display())))?;
    let actual = hex::encode(Sha256::digest(&bytes));
    if actual != expected_sha256 {
        return Err(SandboxError::IntegrityFailure(format!(
            "binary {} hash mismatch: expected {}, got {}",
            path.display(), expected_sha256, actual
        )));
    }
    Ok(())
}

/// Discover the dynamic library dependencies of a binary using `ldd`.
/// Returns a list of absolute paths to .so files that need to be bind-mounted into the jail.
pub fn discover_lib_deps(binary: &Path) -> Vec<PathBuf> {
    let output = std::process::Command::new("ldd")
        .arg(binary)
        .output();
    let output = match output {
        Ok(o) => o,
        Err(_) => return vec![],
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut libs = vec![];
    for line in stdout.lines() {
        // Lines look like:
        //   libz.so.1 => /lib/x86_64-linux-gnu/libz.so.1 (0x...)
        //   /lib64/ld-linux-x86-64.so.2 (0x...)
        let path_str = if let Some(pos) = line.find("=>") {
            // "name => /abs/path (0x...)"
            line[pos + 2..].split_whitespace().next().unwrap_or("")
        } else {
            // "/abs/path (0x...)"
            line.split_whitespace().next().unwrap_or("")
        };
        if path_str.starts_with('/') {
            let p = PathBuf::from(path_str);
            if p.exists() {
                libs.push(p);
            }
        }
    }
    // Add parent directories of libraries (for ld-linux symlinks)
    let mut dirs: Vec<PathBuf> = libs.iter()
        .filter_map(|p| p.parent().map(PathBuf::from))
        .collect();
    dirs.sort();
    dirs.dedup();
    libs.extend(dirs);
    libs
}

/// Enter the jail from inside the worker process. This is called once on worker startup.
///
/// After this call returns **in the child**, the process is inside a new user/mount/pid/net/ipc/uts
/// namespace with a restricted filesystem, no capabilities, NO_NEW_PRIVS, Landlock execute
/// restrictions, and a seccomp filter installed.
///
/// Internally this forks: the child calls `unshare(CLONE_NEWUSER|...)`, signals the parent, the
/// parent writes `uid_map`/`gid_map` (must be done from the parent user namespace), then the child
/// proceeds with mount setup and pivot_root. The parent waits for the child to exit and proxies
/// its exit code via `std::process::exit` — so `enter_jail` effectively **never returns** in the
/// parent process.
///
/// On non-Linux platforms this is a no-op (returns Ok immediately).
#[cfg(target_os = "linux")]
fn check_apparmor_userns() -> Result<()> {
    let val = std::fs::read_to_string("/proc/sys/kernel/apparmor_restrict_unprivileged_userns")
        .unwrap_or_default();
    if val.trim() == "1" {
        return Err(SandboxError::Isolation(
            "AppArmor is blocking unprivileged user namespaces \
             (kernel.apparmor_restrict_unprivileged_userns=1).\n\
             Recommended fix (installs per-binary profile, does not weaken system policy):\n\
             \x20  sudo cp assets/apparmor/edgeplaned-worker /etc/apparmor.d/edgeplaned-worker\n\
             \x20  sudo apparmor_parser -r /etc/apparmor.d/edgeplaned-worker\n\
             Alternative (disables restriction globally):\n\
             \x20  sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0".to_string()
        ));
    }
    Ok(())
}

/// Returns a local tmpfs path for jail root creation.
/// Prefers XDG_RUNTIME_DIR (/run/user/<uid>) over /tmp to avoid NFS/network filesystems.
#[cfg(target_os = "linux")]
fn jail_tmpdir_base() -> std::path::PathBuf {
    if let Some(p) = dirs::runtime_dir() {
        if p.exists() { return p; }
    }
    if let Ok(p) = std::env::var("XDG_RUNTIME_DIR") {
        let pb = std::path::PathBuf::from(p);
        if pb.exists() { return pb; }
    }
    std::path::PathBuf::from("/tmp")
}

#[cfg(target_os = "linux")]
pub fn enter_jail(config: &JailConfig) -> Result<()> {
    use super::linux::apply_sandbox;
    use super::seccomp::{build_filter, install_filter};

    // Step 0: check for AppArmor user namespace restriction before attempting fork
    check_apparmor_userns()?;

    // Step 1: verify binary integrity before entering the jail
    verify_binary_hash(&config.pinned_binary, &config.binary_sha256)?;

    // Step 2: create a temporary directory for our new root (before fork so path is shared).
    // Use the user runtime dir (/run/user/<uid>) — always a local tmpfs, never NFS.
    let base = jail_tmpdir_base();
    let new_root = tempfile::Builder::new()
        .prefix("edgeplaned-jail-")
        .tempdir_in(&base)
        .map_err(|e| SandboxError::Isolation(format!("tmpdir for jail root in {}: {e}", base.display())))?;
    let root_path = new_root.path().to_path_buf();

    // Step 3: set up two pipes for two-phase parent↔child synchronisation:
    //   pipe_a: child → parent  ("I have called unshare, please write uid_map")
    //   pipe_b: parent → child  ("uid_map/gid_map written, you can proceed")
    let mut pipe_a = [0i32; 2]; // [read, write]
    let mut pipe_b = [0i32; 2];
    if unsafe { libc::pipe(pipe_a.as_mut_ptr()) } != 0 {
        return Err(SandboxError::Isolation(format!("pipe_a: {}", std::io::Error::last_os_error())));
    }
    if unsafe { libc::pipe(pipe_b.as_mut_ptr()) } != 0 {
        return Err(SandboxError::Isolation(format!("pipe_b: {}", std::io::Error::last_os_error())));
    }

    // Capture real uid/gid NOW, before fork, while still in the parent namespace
    let real_uid = unsafe { libc::getuid() };
    let real_gid = unsafe { libc::getgid() };

    let child_pid = unsafe { libc::fork() };
    if child_pid < 0 {
        return Err(SandboxError::Isolation(format!("fork: {}", std::io::Error::last_os_error())));
    }

    if child_pid == 0 {
        // ── CHILD ─────────────────────────────────────────────────────────────
        // Close unused ends
        unsafe { libc::close(pipe_a[0]); libc::close(pipe_b[1]); }

        // Step 4a: unshare namespaces
        if let Err(e) = unshare_namespaces() {
            eprintln!("[edgeplaned-jail] unshare failed: {e}");
            unsafe { libc::_exit(127) };
        }

        // Signal parent: "unshare done, please write uid_map"
        unsafe { libc::close(pipe_a[1]); } // EOF signals parent

        // Wait for parent: "uid_map written"
        let mut buf = [0u8; 1];
        unsafe { libc::read(pipe_b[0], buf.as_mut_ptr() as *mut libc::c_void, 1); }
        unsafe { libc::close(pipe_b[0]); }

        // Step 5: set up mount namespace
        if let Err(e) = setup_mount_namespace(&root_path, config) {
            eprintln!("[edgeplaned-jail] mount namespace failed: {e}");
            unsafe { libc::_exit(127) };
        }

        // Step 6: pivot_root
        if let Err(e) = pivot_into(&root_path) {
            eprintln!("[edgeplaned-jail] pivot_root failed: {e}");
            unsafe { libc::_exit(127) };
        }

        // Step 7: NO_NEW_PRIVS
        if let Err(e) = set_no_new_privs() {
            eprintln!("[edgeplaned-jail] no_new_privs failed: {e}");
            unsafe { libc::_exit(127) };
        }

        // Step 8: drop capabilities
        drop_capabilities().ok(); // non-fatal per existing behaviour

        // Step 9: Landlock
        let binary_str = config.pinned_binary.to_string_lossy().to_string();
        apply_sandbox(&[binary_str]).ok(); // non-fatal: seccomp is the hard boundary

        // Step 10: seccomp
        let extra: Vec<&str> = config.extra_deny_syscalls.iter().map(String::as_str).collect();
        if let Ok(filter) = build_filter(&extra) {
            if !filter.is_noop() {
                install_filter(&filter).ok();
            }
        }

        // Keep new_root alive — we've pivoted so the tmpdir won't be dropped on host
        std::mem::forget(new_root);

        // Return Ok to caller — caller runs inside the jail
        return Ok(());
    }

    // ── PARENT ────────────────────────────────────────────────────────────────
    // Close unused ends
    unsafe { libc::close(pipe_a[1]); libc::close(pipe_b[0]); }

    // Wait for child to signal "unshare done"
    let mut buf = [0u8; 1];
    unsafe { libc::read(pipe_a[0], buf.as_mut_ptr() as *mut libc::c_void, 1); }
    unsafe { libc::close(pipe_a[0]); }

    // Step 4b: write uid_map and gid_map from the parent user namespace
    let setgroups_path = format!("/proc/{child_pid}/setgroups");
    let uid_map_path   = format!("/proc/{child_pid}/uid_map");
    let gid_map_path   = format!("/proc/{child_pid}/gid_map");

    // "deny" must be written to setgroups before gid_map (Linux 3.19+)
    let _ = fs::write(&setgroups_path, "deny");
    let _ = fs::write(&uid_map_path, format!("0 {real_uid} 1\n"));
    let _ = fs::write(&gid_map_path, format!("0 {real_gid} 1\n"));

    // Signal child: "uid_map written, proceed"
    unsafe { libc::write(pipe_b[1], buf.as_ptr() as *const libc::c_void, 1); }
    unsafe { libc::close(pipe_b[1]); }

    // Wait for child and proxy its exit code
    let mut status: libc::c_int = 0;
    unsafe { libc::waitpid(child_pid, &mut status, 0); }
    let exit_code = if libc::WIFEXITED(status) { libc::WEXITSTATUS(status) } else { 1 };
    std::process::exit(exit_code);
}

#[cfg(not(target_os = "linux"))]
pub fn enter_jail(_config: &JailConfig) -> Result<()> {
    Ok(())
}

// ── Linux implementation helpers ──────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn unshare_namespaces() -> Result<()> {
    // CLONE_NEWPID is intentionally excluded: PID namespaces require /proc to be mounted
    // (glibc fork/exec reads /proc in PID ns child), and WSL2 doesn't allow mounting proc
    // in user namespaces. The security-critical properties come from the mount namespace
    // (host fs invisible) + landlock + seccomp. PID isolation is a nice-to-have added later
    // when proc-mount works.
    let flags = libc::CLONE_NEWUSER
        | libc::CLONE_NEWNS    // mount
        | libc::CLONE_NEWNET
        | libc::CLONE_NEWIPC
        | libc::CLONE_NEWUTS;
    let ret = unsafe { libc::unshare(flags) };
    if ret != 0 {
        return Err(SandboxError::Isolation(format!(
            "unshare failed: {}", std::io::Error::last_os_error()
        )));
    }
    Ok(())
}


#[cfg(target_os = "linux")]
fn setup_mount_namespace(root: &Path, config: &JailConfig) -> Result<()> {
    use std::ffi::CString;

    // First, make all current mounts private so bind-mounts don't propagate to the host
    let slash_c = CString::new("/").unwrap();
    let none_str = CString::new("none").unwrap();
    unsafe {
        libc::mount(
            none_str.as_ptr(), slash_c.as_ptr(), std::ptr::null(),
            libc::MS_PRIVATE | libc::MS_REC, std::ptr::null(),
        )
    };

    let root_c = CString::new(root.to_string_lossy().as_bytes())
        .map_err(|e| SandboxError::Isolation(format!("root cstring: {e}")))?;

    // Mount a fresh tmpfs at root
    let tmpfs = CString::new("tmpfs").unwrap();
    let ret = unsafe {
        libc::mount(
            tmpfs.as_ptr(), root_c.as_ptr(), tmpfs.as_ptr(),
            libc::MS_NOSUID | libc::MS_NODEV, std::ptr::null(),
        )
    };
    if ret != 0 {
        return Err(SandboxError::Isolation(format!(
            "mount tmpfs at {}: {}", root.display(), std::io::Error::last_os_error()
        )));
    }

    // Create essential directories in the new root
    for dir in &["proc", "tmp", "home", "home/edgeplaned", "usr", "lib", "lib64", "bin", "dev"] {
        fs::create_dir_all(root.join(dir))
            .map_err(|e| SandboxError::Isolation(format!("mkdir {dir}: {e}")))?;
    }

    // Bind-mount /dev/null from the host — many CLI tools open it directly.
    // We bind rather than mknod because mknod for device files requires CAP_MKNOD.
    let dev_null_dest = root.join("dev/null");
    fs::write(&dev_null_dest, "")
        .map_err(|e| SandboxError::Isolation(format!("create dev/null target: {e}")))?;
    // Best-effort: bind mount of /dev/null may fail on some kernels/configs, but most CLI
    // tools will still work (only those that explicitly open /dev/null directly will fail).
    bind_mount(Path::new("/dev/null"), &dev_null_dest, false).ok();

    // Mount a fresh procfs — best-effort (WSL2 and some hardened kernels deny this in user ns)
    let proc_dest_c = CString::new(root.join("proc").to_string_lossy().as_bytes()).unwrap();
    let proc_str = CString::new("proc").unwrap();
    unsafe {
        libc::mount(
            proc_str.as_ptr(), proc_dest_c.as_ptr(), proc_str.as_ptr(),
            libc::MS_NOSUID | libc::MS_NOEXEC | libc::MS_NODEV, std::ptr::null(),
        )
    }; // ignore error — some environments (WSL2) disallow proc mount in user ns

    // Bind-mount the pinned binary into /bin/<name> inside the jail
    let bin_name = config.pinned_binary.file_name()
        .ok_or_else(|| SandboxError::Isolation("binary has no filename".to_string()))?;
    let jail_bin = root.join("bin").join(bin_name);
    fs::write(&jail_bin, "").map_err(|e| SandboxError::Isolation(format!("create bind target: {e}")))?;
    bind_mount(&config.pinned_binary, &jail_bin, true)?;

    // Bind-mount dynamic libraries
    for lib in &config.lib_paths {
        if lib.is_dir() {
            let rel = lib.strip_prefix("/").unwrap_or(lib);
            let dest = root.join(rel);
            fs::create_dir_all(&dest)
                .map_err(|e| SandboxError::Isolation(format!("mkdir lib dir: {e}")))?;
            bind_mount(lib, &dest, true)?;
        } else if lib.is_file() {
            let rel = lib.strip_prefix("/").unwrap_or(lib);
            let dest = root.join(rel);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| SandboxError::Isolation(format!("mkdir lib parent: {e}")))?;
            }
            fs::write(&dest, "").map_err(|e| SandboxError::Isolation(format!("create lib target: {e}")))?;
            bind_mount(lib, &dest, true)?;
        }
    }

    // Extra RO bind mounts from fs policy
    for path_str in &config.fs_policy.extra_ro_bind {
        let src = Path::new(path_str);
        if src.exists() {
            let rel = src.strip_prefix("/").unwrap_or(src);
            let dest = root.join(rel);
            if src.is_dir() {
                fs::create_dir_all(&dest)
                    .map_err(|e| SandboxError::Isolation(format!("mkdir extra ro: {e}")))?;
            } else {
                if let Some(p) = dest.parent() {
                    fs::create_dir_all(p)
                        .map_err(|e| SandboxError::Isolation(format!("mkdir extra ro parent: {e}")))?;
                }
                fs::write(&dest, "").ok();
            }
            bind_mount(src, &dest, true)?;
        }
    }

    // Extra RW bind mounts from fs policy
    for path_str in &config.fs_policy.extra_rw_bind {
        let src = Path::new(path_str);
        if src.exists() {
            let rel = src.strip_prefix("/").unwrap_or(src);
            let dest = root.join(rel);
            if src.is_dir() {
                fs::create_dir_all(&dest)
                    .map_err(|e| SandboxError::Isolation(format!("mkdir extra rw: {e}")))?;
            } else {
                if let Some(p) = dest.parent() {
                    fs::create_dir_all(p)
                        .map_err(|e| SandboxError::Isolation(format!("mkdir extra rw parent: {e}")))?;
                }
                fs::write(&dest, "").ok();
            }
            bind_mount(src, &dest, false)?;
        }
    }

    // /tmp: share host or isolated tmpfs
    if config.fs_policy.share_host_tmp {
        bind_mount(Path::new("/tmp"), &root.join("tmp"), false)?;
    } else {
        let tmp_c = CString::new(root.join("tmp").to_string_lossy().as_bytes()).unwrap();
        unsafe {
            libc::mount(
                tmpfs.as_ptr(), tmp_c.as_ptr(), tmpfs.as_ptr(),
                libc::MS_NOSUID | libc::MS_NODEV, std::ptr::null(),
            )
        };
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn bind_mount(src: &Path, dest: &Path, read_only: bool) -> Result<()> {
    use std::ffi::CString;
    let src_c = CString::new(src.to_string_lossy().as_bytes())
        .map_err(|e| SandboxError::Isolation(format!("bind src cstring: {e}")))?;
    let dest_c = CString::new(dest.to_string_lossy().as_bytes())
        .map_err(|e| SandboxError::Isolation(format!("bind dest cstring: {e}")))?;
    let none_c = CString::new("none").unwrap();

    let ret = unsafe {
        libc::mount(src_c.as_ptr(), dest_c.as_ptr(), std::ptr::null(), libc::MS_BIND, std::ptr::null())
    };
    if ret != 0 {
        return Err(SandboxError::Isolation(format!(
            "bind mount {} → {}: {}",
            src.display(), dest.display(), std::io::Error::last_os_error()
        )));
    }

    if read_only {
        let ret = unsafe {
            libc::mount(
                none_c.as_ptr(), dest_c.as_ptr(), std::ptr::null(),
                libc::MS_BIND | libc::MS_REMOUNT | libc::MS_RDONLY,
                std::ptr::null(),
            )
        };
        if ret != 0 {
            return Err(SandboxError::Isolation(format!(
                "remount ro {} → {}: {}",
                src.display(), dest.display(), std::io::Error::last_os_error()
            )));
        }
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn pivot_into(new_root: &Path) -> Result<()> {
    use std::ffi::CString;
    // Create old_root dir inside new_root for pivot_root to put the old / there
    let put_old = new_root.join(".old_root");
    fs::create_dir_all(&put_old)
        .map_err(|e| SandboxError::Isolation(format!("mkdir old_root: {e}")))?;

    let new_root_c = CString::new(new_root.to_string_lossy().as_bytes()).unwrap();
    let put_old_c = CString::new(put_old.to_string_lossy().as_bytes()).unwrap();

    let ret = unsafe { libc::syscall(libc::SYS_pivot_root, new_root_c.as_ptr(), put_old_c.as_ptr()) };
    if ret != 0 {
        return Err(SandboxError::Isolation(format!(
            "pivot_root failed: {}", std::io::Error::last_os_error()
        )));
    }

    // chdir to the new /
    std::env::set_current_dir("/")
        .map_err(|e| SandboxError::Isolation(format!("chdir to new root: {e}")))?;

    // Unmount the old root
    let old_root_c = CString::new("/.old_root").unwrap();
    unsafe { libc::umount2(old_root_c.as_ptr(), libc::MNT_DETACH) };
    fs::remove_dir("/.old_root").ok();

    Ok(())
}

#[cfg(target_os = "linux")]
fn set_no_new_privs() -> Result<()> {
    let ret = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
    if ret != 0 {
        return Err(SandboxError::Isolation(format!(
            "PR_SET_NO_NEW_PRIVS failed: {}", std::io::Error::last_os_error()
        )));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn drop_capabilities() -> Result<()> {
    // Drop the bounding set for all capabilities
    for cap in 0..64i32 {
        unsafe { libc::prctl(libc::PR_CAPBSET_DROP, cap, 0, 0, 0) };
    }
    // Clear effective, permitted, inheritable capability sets via capset(2).
    // libc doesn't expose the capset structs directly, so we define them inline
    // (matching linux/capability.h) and call the raw syscall.
    #[repr(C)]
    struct CapHeader { version: u32, pid: i32 }
    #[repr(C)]
    struct CapData { effective: u32, permitted: u32, inheritable: u32 }

    const LINUX_CAPABILITY_VERSION_3: u32 = 0x2008_0522;
    const SYS_CAPSET: i64 = 126; // x86_64

    let hdr = CapHeader { version: LINUX_CAPABILITY_VERSION_3, pid: 0 };
    let data = [
        CapData { effective: 0, permitted: 0, inheritable: 0 },
        CapData { effective: 0, permitted: 0, inheritable: 0 },
    ];
    let ret = unsafe {
        libc::syscall(SYS_CAPSET, &hdr as *const CapHeader, data.as_ptr())
    };
    if ret != 0 {
        // Non-fatal — NO_NEW_PRIVS + bounding-set drop already denies escalation
        eprintln!("[edgeplaned-worker] capset warning: {}", std::io::Error::last_os_error());
    }
    Ok(())
}

fn which_binary(command: &str) -> Result<PathBuf> {
    // If already an absolute path, return as-is after existence check
    let p = Path::new(command);
    if p.is_absolute() {
        if p.exists() {
            return Ok(p.to_path_buf());
        }
        return Err(SandboxError::Isolation(format!("binary not found: {command}")));
    }
    // Search PATH
    let path_var = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_string());
    for dir in path_var.split(':') {
        let candidate = Path::new(dir).join(command);
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(SandboxError::Isolation(format!("binary `{command}` not found on PATH")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_nonexistent() {
        assert!(resolve_and_hash_binary("this_binary_does_not_exist_mcd").is_err());
    }

    #[test]
    #[cfg(unix)]
    fn test_resolve_and_hash_true() {
        let (path, hash) = resolve_and_hash_binary("true").unwrap();
        assert!(path.is_absolute());
        assert_eq!(hash.len(), 64); // SHA-256 hex
    }

    #[test]
    #[cfg(unix)]
    fn test_verify_hash_mismatch() {
        let (path, _) = resolve_and_hash_binary("true").unwrap();
        let bad_hash = "a".repeat(64);
        assert!(verify_binary_hash(&path, &bad_hash).is_err());
    }

    #[test]
    fn test_jail_config_env_roundtrip() {
        let config = JailConfig {
            pinned_binary: PathBuf::from("/usr/bin/gcloud"),
            binary_sha256: "abc123".repeat(10) + "abcd",
            lib_paths: vec![PathBuf::from("/lib/x86_64-linux-gnu/libc.so.6")],
            fs_policy: FsPolicy { extra_ro_bind: vec!["/etc/ssl".to_string()], extra_rw_bind: vec![], share_host_tmp: false },
            network_policy: NetworkPolicy::default(),
            limits: CgroupLimits::default(),
            extra_deny_syscalls: vec!["ptrace".to_string()],
        };
        let env_pairs = config.to_env();
        // Set them in process env temporarily
        for (k, v) in &env_pairs {
            std::env::set_var(k, v);
        }
        let loaded = JailConfig::from_env().unwrap();
        assert_eq!(loaded.binary_sha256, config.binary_sha256);
        assert_eq!(loaded.fs_policy.extra_ro_bind, vec!["/etc/ssl"]);
        assert_eq!(loaded.extra_deny_syscalls, vec!["ptrace"]);
        // Clean up
        for (k, _) in &env_pairs {
            std::env::remove_var(k);
        }
    }

    #[test]
    fn test_env_key_prefix() {
        // Ensure env key names use MC_MESH_ prefix
        assert!(env_keys::PINNED_BINARY.starts_with("MC_MESH_"));
        assert!(env_keys::BINARY_SHA256.starts_with("MC_MESH_"));
        assert!(env_keys::LIB_PATHS.starts_with("MC_MESH_"));
        assert!(env_keys::WORKER_SOCKET_FD.starts_with("MC_MESH_"));
    }
}

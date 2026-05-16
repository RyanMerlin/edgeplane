/// Seccomp filter compilation for jailed workers.
///
/// We use a deny-list approach layered over a broad "safe" baseline, rather than a full
/// allowlist, to avoid breaking diverse CLI tools (gcloud, kubectl, etc.) that call a wide
/// variety of syscalls. The baseline blocks the highest-risk syscalls unconditionally;
/// capability manifests can add further denials via `sandbox_profile.extra_syscalls`.
///
/// For each blocked syscall the filter returns ENOSYS (errno 38). This is intentional: many
/// programs handle ENOSYS gracefully, whereas EPERM causes some to crash with confusing errors.
///
/// Requires Linux 3.5+ (seccomp BPF).
use crate::error::{Result, SandboxError};

/// Syscalls that are blocked regardless of capability configuration. These are the highest-risk
/// operations that a CLI tool has no legitimate need for inside the worker jail.
const BASELINE_DENY: &[&str] = &[
    // Kernel / system reconfiguration
    "kexec_load",
    "kexec_file_load",
    "reboot",
    "syslog",
    // Privilege escalation
    "setuid",
    "setgid",
    "setresuid",
    "setresgid",
    "setfsuid",
    "setfsgid",
    "capset",
    "prctl",         // blocked unconditionally — all needed prctl calls (NO_NEW_PRIVS, CAPBSET_DROP) execute before seccomp is installed
    // Namespace creation (workers may not create new namespaces)
    // Note: unshare is blocked to prevent new namespace creation.
    // clone is NOT blocked here — it is needed for legitimate subprocess execution (fork+exec).
    // The user namespace restriction from enter_jail (no uid_map) prevents privilege escalation.
    "unshare",
    // Tracing / injection
    "ptrace",
    "process_vm_readv",
    "process_vm_writev",
    // BPF program loading
    "bpf",
    // Mounting
    "mount",
    "umount2",
    "pivot_root",
    "chroot",
    // Module loading
    "init_module",
    "finit_module",
    "delete_module",
    // Device nodes / udev
    "mknod",
    "mknodat",
    // Kernel keyring
    "add_key",
    "request_key",
    "keyctl",
    // Perf / eBPF observability (deny to prevent side-channel)
    "perf_event_open",
    "lookup_dcookie",
    // Filesystem quotas
    "quotactl",
    // Time manipulation
    "settimeofday",
    "adjtimex",
    "clock_settime",
    "clock_adjtime",
    // IPC namespace operations
    "msgget",
    "msgsnd",
    "msgrcv",
    "msgctl",
    "semget",
    "semop",
    "semctl",
    "shmget",
    "shmat",
    "shmdt",
    "shmctl",
    // NUMA policy
    "set_mempolicy",
    "migrate_pages",
    "move_pages",
    // i/o_uring (large attack surface)
    "io_uring_setup",
    "io_uring_enter",
    "io_uring_register",
];

/// A compiled seccomp filter ready to be installed with `prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER)`.
pub struct SeccompFilter {
    /// Raw BPF instructions.
    pub bpf: Vec<libc::sock_filter>,
}

/// Build a seccomp BPF filter that denies the baseline + any extra syscalls listed in
/// `extra_deny`. All other syscalls are allowed (SECCOMP_RET_ALLOW).
///
/// The filter is structured as:
///   validate arch → for each denied syscall: if nr == X → ENOSYS → else ALLOW
///
/// This is a simple linear scan. For large deny lists a jump table would be faster, but for
/// our baseline (~50 syscalls) the overhead is negligible.
#[cfg(target_arch = "x86_64")]
pub fn build_filter(extra_deny: &[&str]) -> Result<SeccompFilter> {
    use std::collections::HashSet;

    let mut denied: HashSet<i64> = HashSet::new();
    for name in BASELINE_DENY.iter().chain(extra_deny.iter()) {
        if let Some(nr) = syscall_nr(name) {
            denied.insert(nr);
        }
    }

    // BPF constants
    const BPF_LD: u16 = 0x00;
    const BPF_W: u16 = 0x00;
    const BPF_ABS: u16 = 0x20;
    const BPF_JMP: u16 = 0x05;
    const BPF_JEQ: u16 = 0x10;
    const BPF_K: u16 = 0x00;
    const BPF_RET: u16 = 0x06;

    const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
    const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
    const ENOSYS: u32 = 38;

    // Offset of nr field in seccomp_data struct
    const SECCOMP_DATA_NR_OFFSET: u32 = 0;
    const SECCOMP_DATA_ARCH_OFFSET: u32 = 4;
    const AUDIT_ARCH_X86_64: u32 = 0xc000_003e;

    let mut insns: Vec<libc::sock_filter> = vec![
        // Validate arch — if not x86_64, kill the process.
        bpf_stmt(BPF_LD | BPF_W | BPF_ABS, SECCOMP_DATA_ARCH_OFFSET),
        bpf_jump(BPF_JMP | BPF_JEQ | BPF_K, AUDIT_ARCH_X86_64, 1, 0),
        bpf_stmt(BPF_RET | BPF_K, 0x0000_0000u32), // SECCOMP_RET_KILL_PROCESS
        // Load syscall number.
        bpf_stmt(BPF_LD | BPF_W | BPF_ABS, SECCOMP_DATA_NR_OFFSET),
    ];

    // For each denied syscall: if nr == X → return ENOSYS
    let denied_list: Vec<i64> = {
        let mut v: Vec<i64> = denied.into_iter().collect();
        v.sort_unstable();
        v
    };

    for nr in &denied_list {
        if *nr < 0 || *nr > i32::MAX as i64 { continue; }
        insns.push(bpf_jump(BPF_JMP | BPF_JEQ | BPF_K, *nr as u32, 0, 1));
        insns.push(bpf_stmt(BPF_RET | BPF_K, SECCOMP_RET_ERRNO | (ENOSYS & 0xffff)));
    }

    // Default: allow
    insns.push(bpf_stmt(BPF_RET | BPF_K, SECCOMP_RET_ALLOW));

    Ok(SeccompFilter { bpf: insns })
}

/// Install the filter on the calling thread. Must be called after `prctl(PR_SET_NO_NEW_PRIVS, 1)`.
pub fn install_filter(filter: &SeccompFilter) -> Result<()> {
    let prog = libc::sock_fprog {
        len: filter.bpf.len() as u16,
        filter: filter.bpf.as_ptr() as *mut _,
    };
    let ret = unsafe {
        libc::prctl(
            libc::PR_SET_SECCOMP,
            libc::SECCOMP_MODE_FILTER as libc::c_ulong,
            &prog as *const _ as libc::c_ulong,
            0,
            0,
        )
    };
    if ret != 0 {
        return Err(SandboxError::Sandbox(format!(
            "prctl(PR_SET_SECCOMP) failed: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}

#[cfg(not(target_arch = "x86_64"))]
pub fn build_filter(_extra_deny: &[&str]) -> Result<SeccompFilter> {
    // On non-x86_64 architectures we return an empty filter (no-op).
    // The caller should check SeccompFilter::is_noop() before installing.
    Ok(SeccompFilter { bpf: vec![] })
}

impl SeccompFilter {
    pub fn is_noop(&self) -> bool { self.bpf.is_empty() }
}

// ── BPF instruction constructors ──────────────────────────────────────────────

fn bpf_stmt(code: u16, k: u32) -> libc::sock_filter {
    libc::sock_filter { code, jt: 0, jf: 0, k }
}

fn bpf_jump(code: u16, k: u32, jt: u8, jf: u8) -> libc::sock_filter {
    libc::sock_filter { code, jt, jf, k }
}

// ── x86_64 syscall number table (subset) ──────────────────────────────────────

#[cfg(target_arch = "x86_64")]
fn syscall_nr(name: &str) -> Option<i64> {
    // Minimal lookup table for the syscalls we reference. Generated from unistd.h.
    let table: &[(&str, i64)] = &[
        ("read", 0), ("write", 1), ("open", 2), ("close", 3), ("stat", 4), ("fstat", 5),
        ("lstat", 6), ("poll", 7), ("lseek", 8), ("mmap", 9), ("mprotect", 10),
        ("munmap", 11), ("brk", 12), ("rt_sigaction", 13), ("rt_sigprocmask", 14),
        ("rt_sigreturn", 15), ("ioctl", 16), ("pread64", 17), ("pwrite64", 18),
        ("readv", 19), ("writev", 20), ("access", 21), ("pipe", 22), ("select", 23),
        ("sched_yield", 24), ("mremap", 25), ("msync", 26), ("mincore", 27),
        ("madvise", 28), ("shmget", 29), ("shmat", 30), ("shmctl", 31), ("dup", 32),
        ("dup2", 33), ("pause", 34), ("nanosleep", 35), ("getitimer", 36),
        ("alarm", 37), ("setitimer", 38), ("getpid", 39), ("sendfile", 40),
        ("socket", 41), ("connect", 42), ("accept", 43), ("sendto", 44),
        ("recvfrom", 45), ("sendmsg", 46), ("recvmsg", 47), ("shutdown", 48),
        ("bind", 49), ("listen", 50), ("getsockname", 51), ("getpeername", 52),
        ("socketpair", 53), ("setsockopt", 54), ("getsockopt", 55), ("clone", 56),
        ("fork", 57), ("vfork", 58), ("execve", 59), ("exit", 60), ("wait4", 61),
        ("kill", 62), ("uname", 63), ("semget", 64), ("semop", 65), ("semctl", 66),
        ("shmdt", 67), ("msgget", 68), ("msgsnd", 69), ("msgrcv", 70), ("msgctl", 71),
        ("fcntl", 72), ("flock", 73), ("fsync", 74), ("fdatasync", 75), ("truncate", 76),
        ("ftruncate", 77), ("getdents", 78), ("getcwd", 79), ("chdir", 80),
        ("fchdir", 81), ("rename", 82), ("mkdir", 83), ("rmdir", 84), ("creat", 85),
        ("link", 86), ("unlink", 87), ("symlink", 88), ("readlink", 89), ("chmod", 90),
        ("fchmod", 91), ("chown", 92), ("fchown", 93), ("lchown", 94), ("umask", 95),
        ("gettimeofday", 96), ("getrlimit", 97), ("getrusage", 98), ("sysinfo", 99),
        ("times", 100), ("ptrace", 101), ("getuid", 102), ("syslog", 103),
        ("getgid", 104), ("setuid", 105), ("setgid", 106), ("geteuid", 107),
        ("getegid", 108), ("setpgid", 109), ("getppid", 110), ("getpgrp", 111),
        ("setsid", 112), ("setreuid", 113), ("setregid", 114), ("getgroups", 115),
        ("setgroups", 116), ("setresuid", 117), ("getresuid", 118), ("setresgid", 119),
        ("getresgid", 120), ("getpgid", 121), ("setfsuid", 122), ("setfsgid", 123),
        ("getsid", 124), ("capget", 125), ("capset", 126), ("rt_sigpending", 127),
        ("rt_sigtimedwait", 128), ("rt_sigqueueinfo", 129), ("rt_sigsuspend", 130),
        ("sigaltstack", 131), ("utime", 132), ("mknod", 133), ("uselib", 134),
        ("personality", 135), ("ustat", 136), ("statfs", 137), ("fstatfs", 138),
        ("sysfs", 139), ("getpriority", 140), ("setpriority", 141), ("sched_setparam", 142),
        ("sched_getparam", 143), ("sched_setscheduler", 144), ("sched_getscheduler", 145),
        ("sched_get_priority_max", 146), ("sched_get_priority_min", 147),
        ("sched_rr_get_interval", 148), ("mlock", 149), ("munlock", 150),
        ("mlockall", 151), ("munlockall", 152), ("vhangup", 153), ("modify_ldt", 154),
        ("pivot_root", 155), ("_sysctl", 156), ("prctl", 157), ("arch_prctl", 158),
        ("adjtimex", 159), ("setrlimit", 160), ("chroot", 161), ("sync", 162),
        ("acct", 163), ("settimeofday", 164), ("mount", 165), ("umount2", 166),
        ("swapon", 167), ("swapoff", 168), ("reboot", 169), ("sethostname", 170),
        ("setdomainname", 171), ("iopl", 172), ("ioperm", 173), ("create_module", 174),
        ("init_module", 175), ("delete_module", 176), ("get_kernel_syms", 177),
        ("query_module", 178), ("quotactl", 179), ("nfsservctl", 180), ("getpmsg", 181),
        ("putpmsg", 182), ("afs_syscall", 183), ("tuxcall", 184), ("security", 185),
        ("gettid", 186), ("readahead", 187), ("setxattr", 188), ("lsetxattr", 189),
        ("fsetxattr", 190), ("getxattr", 191), ("lgetxattr", 192), ("fgetxattr", 193),
        ("listxattr", 194), ("llistxattr", 195), ("flistxattr", 196),
        ("removexattr", 197), ("lremovexattr", 198), ("fremovexattr", 199),
        ("tkill", 200), ("time", 201), ("futex", 202), ("sched_setaffinity", 203),
        ("sched_getaffinity", 204), ("set_thread_area", 205), ("io_setup", 206),
        ("io_destroy", 207), ("io_getevents", 208), ("io_submit", 209),
        ("io_cancel", 210), ("get_thread_area", 211), ("lookup_dcookie", 212),
        ("epoll_create", 213), ("epoll_ctl_old", 214), ("epoll_wait_old", 215),
        ("remap_file_pages", 216), ("getdents64", 217), ("set_tid_address", 218),
        ("restart_syscall", 219), ("semtimedop", 220), ("fadvise64", 221),
        ("timer_create", 222), ("timer_settime", 223), ("timer_gettime", 224),
        ("timer_getoverrun", 225), ("timer_delete", 226), ("clock_settime", 227),
        ("clock_gettime", 228), ("clock_getres", 229), ("clock_nanosleep", 230),
        ("exit_group", 231), ("epoll_wait", 232), ("epoll_ctl", 233), ("tgkill", 234),
        ("utimes", 235), ("vserver", 236), ("mbind", 237), ("set_mempolicy", 238),
        ("get_mempolicy", 239), ("mq_open", 240), ("mq_unlink", 241),
        ("mq_timedsend", 242), ("mq_timedreceive", 243), ("mq_notify", 244),
        ("mq_getsetattr", 245), ("kexec_load", 246), ("waitid", 247),
        ("add_key", 248), ("request_key", 249), ("keyctl", 250),
        ("ioprio_set", 251), ("ioprio_get", 252), ("inotify_init", 253),
        ("inotify_add_watch", 254), ("inotify_rm_watch", 255), ("migrate_pages", 256),
        ("openat", 257), ("mkdirat", 258), ("mknodat", 259), ("fchownat", 260),
        ("futimesat", 261), ("newfstatat", 262), ("unlinkat", 263), ("renameat", 264),
        ("linkat", 265), ("symlinkat", 266), ("readlinkat", 267), ("fchmodat", 268),
        ("faccessat", 269), ("pselect6", 270), ("ppoll", 271), ("unshare", 272),
        ("set_robust_list", 273), ("get_robust_list", 274), ("splice", 275),
        ("tee", 276), ("sync_file_range", 277), ("vmsplice", 278), ("move_pages", 279),
        ("utimensat", 280), ("epoll_pwait", 281), ("signalfd", 282), ("timerfd_create", 283),
        ("eventfd", 284), ("fallocate", 285), ("timerfd_settime", 286),
        ("timerfd_gettime", 287), ("accept4", 288), ("signalfd4", 289), ("eventfd2", 290),
        ("epoll_create1", 291), ("dup3", 292), ("pipe2", 293), ("inotify_init1", 294),
        ("preadv", 295), ("pwritev", 296), ("rt_tgsigqueueinfo", 297),
        ("perf_event_open", 298), ("recvmmsg", 299), ("fanotify_init", 300),
        ("fanotify_mark", 301), ("prlimit64", 302), ("name_to_handle_at", 303),
        ("open_by_handle_at", 304), ("clock_adjtime", 305), ("syncfs", 306),
        ("sendmmsg", 307), ("setns", 308), ("getcpu", 309), ("process_vm_readv", 310),
        ("process_vm_writev", 311), ("kcmp", 312), ("finit_module", 313),
        ("sched_setattr", 314), ("sched_getattr", 315), ("renameat2", 316),
        ("seccomp", 317), ("getrandom", 318), ("memfd_create", 319),
        ("kexec_file_load", 320), ("bpf", 321), ("execveat", 322),
        ("userfaultfd", 323), ("membarrier", 324), ("mlock2", 325),
        ("copy_file_range", 326), ("preadv2", 327), ("pwritev2", 328),
        ("pkey_mprotect", 329), ("pkey_alloc", 330), ("pkey_free", 331),
        ("statx", 332), ("io_pgetevents", 333), ("rseq", 334),
        ("pidfd_send_signal", 424), ("io_uring_setup", 425), ("io_uring_enter", 426),
        ("io_uring_register", 427), ("open_tree", 428), ("move_mount", 429),
        ("fsopen", 430), ("fsconfig", 431), ("fsmount", 432), ("fspick", 433),
        ("pidfd_open", 434), ("clone3", 435), ("close_range", 436),
        ("openat2", 437), ("pidfd_getfd", 438), ("faccessat2", 439),
        ("process_madvise", 440), ("epoll_pwait2", 441), ("mount_setattr", 442),
        ("quotactl_fd", 443), ("landlock_create_ruleset", 444),
        ("landlock_add_rule", 445), ("landlock_restrict_self", 446),
        ("memfd_secret", 447), ("process_mrelease", 448), ("futex_waitv", 449),
        ("set_mempolicy_home_node", 450),
    ];
    table.iter().find(|(n, _)| *n == name).map(|(_, nr)| *nr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_filter_empty_extra() {
        let f = build_filter(&[]).unwrap();
        #[cfg(target_arch = "x86_64")]
        assert!(!f.bpf.is_empty());
    }

    #[test]
    fn test_syscall_nr_known() {
        #[cfg(target_arch = "x86_64")]
        assert_eq!(syscall_nr("read"), Some(0));
        #[cfg(target_arch = "x86_64")]
        assert_eq!(syscall_nr("bpf"), Some(321));
    }
}

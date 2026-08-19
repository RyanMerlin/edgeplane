//! Kernel-enforced singleton via flock(2) on the edgeplaned lock file.
//!
//! ## Why flock
//!
//! Held for the lifetime of the daemon process. Released by the kernel on any
//! termination — clean exit, panic, SIGKILL, OOM kill, power loss. Stale lock
//! files are harmless because flock state lives in the kernel's open file table,
//! not in the file contents.
//!
//! This is strictly stronger than PID files (stale-file false positives),
//! `kill(pid, 0)` checks (PID-recycle false negatives), or "try to bind a
//! socket" (race between bind and lock). The kernel guarantees atomicity.
//!
//! ## Contract
//!
//! `SingletonLock::acquire()` is called at the very top of `daemon::run()`,
//! before any port binds, registry opens, or controlplane fetches. If another
//! edgeplaned holds the lock, this function returns a structured error that an
//! operator (or an LLM agent reading the error) cannot reasonably ignore.
//!
//! On success, the lock file contents are overwritten with the holder's
//! identity (PID, binary path, start time) — purely informational for
//! debugging. The flock itself is the source of truth.
//!
//! ## Forced takeover (`--kill-existing`)
//!
//! When the existing daemon is hung or zombie, the operator can pass
//! `--kill-existing`. We SIGTERM the holder, poll for lock release for up to
//! 5 seconds, then SIGKILL if still held, then retry the lock acquisition
//! exactly once. If the second attempt also fails, we surface the structured
//! error — there is no infinite retry loop.

use anyhow::{Result, anyhow};
use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Holds the kernel flock for the daemon's lifetime. Drop releases the lock.
#[derive(Debug)]
pub struct SingletonLock {
    _file: File,
    #[allow(dead_code)]
    path: PathBuf,
}

#[derive(Debug, Default)]
struct HolderInfo {
    pid: Option<i32>,
    binary: Option<String>,
    started: Option<String>,
}

impl SingletonLock {
    /// Acquire the singleton lock at `lock_path`.
    ///
    /// On contention:
    /// - `kill_existing == false`: returns a structured error naming the
    ///   holder and the remediation steps.
    /// - `kill_existing == true`: SIGTERM the holder, poll up to 5s for
    ///   release, SIGKILL if needed, retry once.
    pub fn acquire(lock_path: &Path, kill_existing: bool) -> Result<Self> {
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| anyhow!("create lock dir {parent:?}: {e}"))?;
        }

        match Self::try_lock(lock_path) {
            Ok(lock) => Ok(lock),
            Err(LockError::WouldBlock) if kill_existing => {
                let info = read_holder(lock_path);
                let pid = info.pid.ok_or_else(|| {
                    anyhow!(
                        "lock at {lock_path:?} is held but holder PID is unreadable; \
                         cannot --kill-existing safely. Investigate manually."
                    )
                })?;
                eprintln!("--kill-existing: terminating edgeplaned PID {pid}");
                kill_holder(pid)?;
                // Retry once. Any further contention is a real problem.
                match Self::try_lock(lock_path) {
                    Ok(lock) => Ok(lock),
                    Err(LockError::WouldBlock) => Err(anyhow!(
                        "lock at {lock_path:?} still held after --kill-existing \
                         terminated PID {pid}; another edgeplaned may have started in the \
                         meantime. Run `edgeplaned doctor` to inspect."
                    )),
                    Err(LockError::Io(e)) => Err(anyhow!("re-acquire after kill: {e}")),
                }
            }
            Err(LockError::WouldBlock) => Err(anyhow!("{}", format_holder_error(lock_path))),
            Err(LockError::Io(e)) => Err(anyhow!("open lock file {lock_path:?}: {e}")),
        }
    }

    fn try_lock(lock_path: &Path) -> std::result::Result<Self, LockError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .map_err(LockError::Io)?;

        match file.try_lock_exclusive() {
            Ok(()) => {
                write_identity(&file).map_err(LockError::Io)?;
                Ok(Self {
                    _file: file,
                    path: lock_path.to_path_buf(),
                })
            }
            // fs2 maps EAGAIN/EWOULDBLOCK to ErrorKind::WouldBlock on Linux.
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Err(LockError::WouldBlock),
            Err(e) => Err(LockError::Io(e)),
        }
    }
}

enum LockError {
    WouldBlock,
    Io(std::io::Error),
}

fn write_identity(mut file: &File) -> std::io::Result<()> {
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "<unknown>".into());
    let started = chrono::Local::now().to_rfc3339();
    writeln!(
        file,
        "pid={}\nbinary={}\nstarted={}",
        std::process::id(),
        exe,
        started
    )?;
    file.sync_data()?;
    Ok(())
}

fn read_holder(lock_path: &Path) -> HolderInfo {
    let Ok(s) = std::fs::read_to_string(lock_path) else {
        return HolderInfo::default();
    };
    let mut info = HolderInfo::default();
    for line in s.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim().to_string();
        match k.trim() {
            "pid" => info.pid = v.parse().ok(),
            "binary" => info.binary = Some(v),
            "started" => info.started = Some(v),
            _ => {}
        }
    }
    info
}

/// Build the operator-facing error message shown when contention is not being
/// forced through with `--kill-existing`. Deliberately long and explicit —
/// the failure mode this guards against is data-corrupting, so the cost of
/// a verbose error is small relative to the cost of silently launching a
/// parallel daemon.
fn format_holder_error(lock_path: &Path) -> String {
    let info = read_holder(lock_path);
    let pid = info
        .pid
        .map(|p| p.to_string())
        .unwrap_or_else(|| "<unknown>".into());
    let binary = info.binary.unwrap_or_else(|| "<unknown>".into());
    let started = info.started.unwrap_or_else(|| "<unknown>".into());

    format!(
        "another edgeplaned is already running.\n\n  \
         holder PID:     {pid}\n  \
         holder binary:  {binary}\n  \
         holder started: {started}\n  \
         lock file:      {}\n\n\
         This is a singleton daemon. Running two instances corrupts the local \
         registry and double-spawns agents.\n\n\
         To proceed:\n  \
         1. If the holder is healthy:\n       \
              systemctl --user restart edgeplaned.service     # systemd-managed\n       \
              kill -TERM {pid}                          # shell-launched\n  \
         2. If hung or unresponsive:\n       \
              edgeplaned run --kill-existing                   # SIGTERM, then SIGKILL, then take over\n  \
         3. To inspect before deciding:\n       \
              ps -p {pid} -o pid,etime,cmd\n       \
              journalctl --user -u edgeplaned.service -n 50\n       \
              edgeplaned doctor",
        lock_path.display(),
    )
}

fn kill_holder(pid: i32) -> Result<()> {
    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;

    let target = Pid::from_raw(pid);

    // Step 1: SIGTERM
    if let Err(e) = kill(target, Signal::SIGTERM) {
        // ESRCH = no such process — already dead, treat as success.
        if e == nix::errno::Errno::ESRCH {
            return Ok(());
        }
        return Err(anyhow!("SIGTERM PID {pid}: {e}"));
    }

    // Step 2: poll for exit up to 5 seconds.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        match kill(target, None) {
            Err(nix::errno::Errno::ESRCH) => return Ok(()), // gone
            _ => std::thread::sleep(Duration::from_millis(100)),
        }
    }

    // Step 3: SIGKILL.
    eprintln!("--kill-existing: PID {pid} did not exit within 5s; sending SIGKILL");
    if let Err(e) = kill(target, Signal::SIGKILL) {
        if e == nix::errno::Errno::ESRCH {
            return Ok(());
        }
        return Err(anyhow!("SIGKILL PID {pid}: {e}"));
    }

    // Give the kernel a moment to clean up before we retry the lock.
    std::thread::sleep(Duration::from_millis(200));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn acquire_creates_lock_file_and_writes_identity() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("edgeplaned.lock");
        let _lock = SingletonLock::acquire(&path, false).expect("first acquire");
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains(&format!("pid={}", std::process::id())));
        assert!(contents.contains("binary="));
        assert!(contents.contains("started="));
    }

    #[test]
    fn second_acquire_in_same_process_fails_with_holder_info() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("edgeplaned.lock");
        let _first = SingletonLock::acquire(&path, false).expect("first acquire");
        let err = SingletonLock::acquire(&path, false).expect_err("second must fail");
        let msg = format!("{err}");
        assert!(
            msg.contains("another edgeplaned is already running"),
            "msg: {msg}"
        );
        assert!(msg.contains(&format!("holder PID:     {}", std::process::id())));
        assert!(msg.contains("edgeplaned run --kill-existing"));
    }

    #[test]
    fn lock_released_on_drop() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("edgeplaned.lock");
        {
            let _first = SingletonLock::acquire(&path, false).expect("first acquire");
        }
        // Drop released the lock — second acquire must succeed.
        let _second = SingletonLock::acquire(&path, false).expect("post-drop acquire");
    }

    #[test]
    fn read_holder_parses_kv_lines() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("edgeplaned.lock");
        std::fs::write(
            &path,
            "pid=12345\nbinary=/usr/bin/edgeplaned\nstarted=2026-05-17T12:00:00\n",
        )
        .unwrap();
        let info = read_holder(&path);
        assert_eq!(info.pid, Some(12345));
        assert_eq!(info.binary.as_deref(), Some("/usr/bin/edgeplaned"));
        assert_eq!(info.started.as_deref(), Some("2026-05-17T12:00:00"));
    }

    #[test]
    fn read_holder_tolerates_missing_or_malformed() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("missing.lock");
        let info = read_holder(&path);
        assert!(info.pid.is_none());

        let path2 = dir.path().join("garbage.lock");
        std::fs::write(&path2, "not a kv file\n").unwrap();
        let info2 = read_holder(&path2);
        assert!(info2.pid.is_none());
    }
}

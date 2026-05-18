//! `mcd doctor` — read-only health check.
//!
//! Reports four things, in order:
//! 1. Singleton lock state (held? by whom?)
//! 2. Required TCP ports (8009 attach_ws, 7731 mgmt) — bindable or in use?
//! 3. Local SQLite registry — openable for read?
//! 4. Agent runtimes — node / claude / claude-agent-acp resolution.
//!
//! This command never connects to the running daemon's gateways. It exists
//! precisely for the case where the daemon is misbehaving (won't start, lock
//! stuck, port conflict) and an operator needs ground truth from outside.
//!
//! Exit code: 0 if everything is healthy or the only finding is "lock is held
//! by a live mcd" (the expected steady state). Non-zero if anything looks
//! actionably wrong.

use anyhow::Result;
use fs2::FileExt;
use std::fs::OpenOptions;

pub async fn run() -> Result<()> {
    let mut findings: Vec<Finding> = Vec::new();

    findings.push(check_lock());
    findings.extend(check_ports().await);
    findings.push(check_registry());
    findings.extend(check_runtimes());

    println!("mcd doctor — {}", chrono::Local::now().to_rfc3339());
    println!();
    let mut had_error = false;
    for f in &findings {
        let tag = match f.severity {
            Severity::Ok => "  OK   ",
            Severity::Info => " INFO  ",
            Severity::Warn => " WARN  ",
            Severity::Error => " ERROR ",
        };
        println!("[{tag}] {}", f.title);
        for line in f.detail.lines() {
            println!("         {line}");
        }
        if matches!(f.severity, Severity::Error) {
            had_error = true;
        }
    }
    println!();
    if had_error {
        std::process::exit(1);
    }
    Ok(())
}

enum Severity {
    Ok,
    Info,
    Warn,
    Error,
}

struct Finding {
    severity: Severity,
    title: String,
    detail: String,
}

/// Probe the singleton lock without disrupting a healthy holder.
///
/// We open the lock file and attempt a non-blocking exclusive lock. If we
/// get it, no one was holding it — release immediately. If we get
/// `WouldBlock`, someone is holding it: read the file for identity.
fn check_lock() -> Finding {
    let path = mcd_core::paths::lock_file_path();
    let path_display = path.display().to_string();

    if !path.exists() {
        return Finding {
            severity: Severity::Info,
            title: "singleton lock".into(),
            detail: format!(
                "lock file not yet created at {path_display}\n\
                 (expected if mcd has never run; will appear on first start)"
            ),
        };
    }

    let file = match OpenOptions::new().read(true).write(true).open(&path) {
        Ok(f) => f,
        Err(e) => {
            return Finding {
                severity: Severity::Error,
                title: "singleton lock".into(),
                detail: format!("cannot open {path_display}: {e}"),
            };
        }
    };

    match file.try_lock_exclusive() {
        Ok(()) => {
            // We got it — release immediately. Means no daemon is running.
            let _ = FileExt::unlock(&file);
            Finding {
                severity: Severity::Info,
                title: "singleton lock".into(),
                detail: format!(
                    "lock at {path_display} is NOT held — no mcd is currently running.\n\
                     Start with `systemctl --user start mcd.service` or `mcd run`."
                ),
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
            let contents = std::fs::read_to_string(&path).unwrap_or_default();
            Finding {
                severity: Severity::Ok,
                title: "singleton lock".into(),
                detail: format!(
                    "lock at {path_display} is held (a daemon is running).\n\
                     holder identity:\n{}",
                    contents
                        .lines()
                        .map(|l| format!("  {l}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                ),
            }
        }
        Err(e) => Finding {
            severity: Severity::Error,
            title: "singleton lock".into(),
            detail: format!("flock error on {path_display}: {e}"),
        },
    }
}

/// For each required port, try to bind. Three outcomes:
/// - Ok bind → no listener present (daemon down): INFO, not WARN.
/// - AddrInUse → listener present. Check if it's the running mcd via the
///   singleton lock; if yes, this is expected and reported as OK.
/// - Other error → ERROR.
async fn check_ports() -> Vec<Finding> {
    let mgmt_port: u16 = std::env::var("MC_MESH_MGMT_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(7731);

    let ports: [(&str, String); 2] = [
        ("attach_ws", "0.0.0.0:8009".into()),
        ("mgmt_tcp", format!("0.0.0.0:{mgmt_port}")),
    ];

    let mcd_running = lock_is_held();
    let mut out = Vec::with_capacity(ports.len());
    for (name, addr) in ports {
        out.push(check_one_port(name, &addr, mcd_running).await);
    }
    out
}

async fn check_one_port(name: &str, addr: &str, mcd_running: bool) -> Finding {
    match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => {
            drop(listener);
            if mcd_running {
                Finding {
                    severity: Severity::Warn,
                    title: format!("port {name} ({addr})"),
                    detail: format!(
                        "port is free but a daemon appears to be running per the singleton lock.\n\
                         The daemon may have failed to bind this port. \
                         Check `journalctl --user -u mcd.service` for bind errors."
                    ),
                }
            } else {
                Finding {
                    severity: Severity::Info,
                    title: format!("port {name} ({addr})"),
                    detail: "port is free (no daemon listening — consistent with lock state)".into(),
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            if mcd_running {
                Finding {
                    severity: Severity::Ok,
                    title: format!("port {name} ({addr})"),
                    detail: "bound (expected — owned by the running mcd)".into(),
                }
            } else {
                Finding {
                    severity: Severity::Error,
                    title: format!("port {name} ({addr})"),
                    detail: format!(
                        "port is in use but no mcd holds the singleton lock — something \
                         else owns this port.\n\
                         To find it: ss -lntp | grep {addr}"
                    ),
                }
            }
        }
        Err(e) => Finding {
            severity: Severity::Error,
            title: format!("port {name} ({addr})"),
            detail: format!("bind probe failed: {e}"),
        },
    }
}

fn lock_is_held() -> bool {
    let path = mcd_core::paths::lock_file_path();
    if !path.exists() {
        return false;
    }
    let Ok(file) = OpenOptions::new().read(true).write(true).open(&path) else {
        return false;
    };
    match file.try_lock_exclusive() {
        Ok(()) => {
            let _ = FileExt::unlock(&file);
            false
        }
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => true,
        Err(_) => false,
    }
}

fn check_registry() -> Finding {
    let path = mcd_core::paths::registry_db_path();
    let display = path.display().to_string();
    if !path.exists() {
        return Finding {
            severity: Severity::Info,
            title: "local registry".into(),
            detail: format!("{display} does not exist (will be created on first daemon start)"),
        };
    }
    // Try to open read-only via rusqlite — cheap, exclusive locking doesn't
    // apply with default sqlite settings, so this won't disrupt a running daemon.
    match rusqlite::Connection::open_with_flags(
        &path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) {
        Ok(conn) => {
            let count: rusqlite::Result<i64> = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
                [],
                |r| r.get(0),
            );
            match count {
                Ok(n) => Finding {
                    severity: Severity::Ok,
                    title: "local registry".into(),
                    detail: format!("{display} opens cleanly ({n} table(s))"),
                },
                Err(e) => Finding {
                    severity: Severity::Warn,
                    title: "local registry".into(),
                    detail: format!("{display} opens but schema query failed: {e}"),
                },
            }
        }
        Err(e) => Finding {
            severity: Severity::Error,
            title: "local registry".into(),
            detail: format!("cannot open {display}: {e}"),
        },
    }
}

fn check_runtimes() -> Vec<Finding> {
    let mut out = Vec::new();

    out.push(check_binary("node", &["--version"]));
    out.push(check_binary("claude", &["--version"]));

    // claude-agent-acp lives as a node module — look for its dist/index.js
    // in the npm global root. Best-effort.
    out.push(check_acp_module());

    out
}

fn check_binary(name: &str, args: &[&str]) -> Finding {
    match which::which(name) {
        Ok(path) => {
            let version = std::process::Command::new(&path)
                .args(args)
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| "(version probe failed)".into());
            Finding {
                severity: Severity::Ok,
                title: format!("runtime: {name}"),
                detail: format!("{} — {version}", path.display()),
            }
        }
        Err(_) => Finding {
            severity: Severity::Warn,
            title: format!("runtime: {name}"),
            detail: format!("{name} not found in PATH (required for ACP runtimes)"),
        },
    }
}

fn check_acp_module() -> Finding {
    let pkg = "@agentclientprotocol/claude-agent-acp";
    if let Ok(out) = std::process::Command::new("npm").args(["root", "-g"]).output() {
        if out.status.success() {
            let root = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let p = std::path::PathBuf::from(&root).join(format!("{pkg}/dist/index.js"));
            if p.exists() {
                return Finding {
                    severity: Severity::Ok,
                    title: format!("runtime: {pkg}"),
                    detail: p.display().to_string(),
                };
            }
        }
    }
    Finding {
        severity: Severity::Warn,
        title: format!("runtime: {pkg}"),
        detail: format!(
            "{pkg} not found in npm global root.\n\
             Install with: npm install -g {pkg}"
        ),
    }
}

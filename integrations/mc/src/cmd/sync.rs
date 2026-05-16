/// `mc sync` — bidirectional git-backed config sync for the local node.
///
/// Reads sync_repo from MC_SYNC_REPO env or ~/.mc/config.json.
use anyhow::Result;
use clap::Subcommand;

// ---------------------------------------------------------------------------
// Clap types
// ---------------------------------------------------------------------------

#[derive(Subcommand, Debug)]
pub enum SyncCmd {
    /// Pull latest config from sync repo (default action).
    #[command(about = "Pull latest config from sync repo (default action)")]
    Pull,
    /// Show sync status.
    #[command(about = "Show sync status")]
    Status,
    /// Push local node config changes.
    #[command(about = "Push local node config changes")]
    Push {
        #[arg(long, default_value = "Update node config")]
        message: String,
    },
}

// ---------------------------------------------------------------------------
// Config reading
// ---------------------------------------------------------------------------

/// Minimal overlay on top of the persisted config.json to read sync_repo.
#[derive(serde::Deserialize, Default)]
struct McSyncConfig {
    sync_repo: Option<String>,
}

fn read_config_sync_repo() -> Option<String> {
    let path = crate::config::mc_home_dir().join("config.json");
    let content = std::fs::read_to_string(path).ok()?;
    let cfg: McSyncConfig = serde_json::from_str(&content).ok()?;
    cfg.sync_repo
}

fn resolve_hostname() -> String {
    // Use the libc approach via the hostname command — avoids an extra crate dep.
    std::process::Command::new("hostname")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// `cmd` is `None` when the user runs `mc sync` with no subcommand; default to Pull.
pub fn run(cmd: Option<SyncCmd>) -> Result<()> {
    use mcd_sync::SyncClient;

    let repo_url = std::env::var("MC_SYNC_REPO")
        .ok()
        .or_else(read_config_sync_repo)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "sync repo not configured — set MC_SYNC_REPO or add sync_repo to ~/.mc/config.json"
            )
        })?;

    let cache_dir = crate::config::mc_home_dir().join("sync");
    let hostname = resolve_hostname();

    let client = SyncClient::new(&repo_url, &cache_dir, &hostname)?;

    match cmd.unwrap_or(SyncCmd::Pull) {
        SyncCmd::Pull => {
            println!("syncing from {}...", repo_url);
            let result = client.pull()?;
            println!("synced — {} new commits", result.commits_fetched);
        }
        SyncCmd::Status => {
            let status = client.status()?;
            println!(
                "last pulled:  {}",
                status
                    .last_pulled_at
                    .map(|t| t.to_rfc3339())
                    .unwrap_or_else(|| "never".to_string())
            );
            println!(
                "last pushed:  {}",
                status
                    .last_pushed_at
                    .map(|t| t.to_rfc3339())
                    .unwrap_or_else(|| "never".to_string())
            );
            println!(
                "node dirty:   {}",
                if status.node_branch_dirty { "yes" } else { "no" }
            );
            println!("fleet ahead:  {} commits", status.fleet_branch_ahead);
        }
        SyncCmd::Push { message } => {
            let result = client.push_node_changes(&message)?;
            println!(
                "pushed {} files to {}",
                result.files_committed, result.branch
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_run_errors_without_config() {
        // Ensure no env var leaks from parent process.
        // We use a temp HOME dir so config.json won't be found.
        let dir = tempfile::tempdir().unwrap();
        let orig_home = std::env::var("MC_HOME").ok();
        let orig_sync = std::env::var("MC_SYNC_REPO").ok();

        // SAFETY: single-threaded test; no concurrent env access.
        unsafe {
            std::env::set_var("MC_HOME", dir.path().to_str().unwrap());
            std::env::remove_var("MC_SYNC_REPO");
        }

        let result = run(Some(SyncCmd::Status));

        // Restore environment.
        // SAFETY: single-threaded test; no concurrent env access.
        unsafe {
            match orig_home {
                Some(v) => std::env::set_var("MC_HOME", v),
                None => std::env::remove_var("MC_HOME"),
            }
            if let Some(v) = orig_sync {
                std::env::set_var("MC_SYNC_REPO", v);
            }
        }

        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("sync repo not configured"),
            "expected 'sync repo not configured', got: {msg}"
        );
    }
}

use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::{DateTime, Utc};

use crate::error::SyncError;
use crate::types::{PushResult, SyncResult, SyncState, SyncStatus};

pub struct SyncClient {
    repo_url: String,
    cache_dir: PathBuf,
    hostname: String,
}

impl SyncClient {
    pub fn new(repo_url: &str, cache_dir: &Path, hostname: &str) -> Result<Self, SyncError> {
        Ok(Self {
            repo_url: repo_url.to_string(),
            cache_dir: cache_dir.to_path_buf(),
            hostname: hostname.to_string(),
        })
    }

    fn state_file(&self) -> PathBuf {
        // <cache_dir>/../sync-state.json
        self.cache_dir
            .parent()
            .unwrap_or_else(|| Path::new("/tmp"))
            .join("sync-state.json")
    }

    fn read_state(&self) -> Result<SyncState, SyncError> {
        let path = self.state_file();
        if !path.exists() {
            return Ok(SyncState::default());
        }
        let contents = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&contents)?)
    }

    fn write_state(&self, state: &SyncState) -> Result<(), SyncError> {
        let path = self.state_file();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, serde_json::to_string_pretty(state)?)?;
        Ok(())
    }

    fn git(&self, args: &[&str]) -> Result<String, SyncError> {
        let output = Command::new("git").args(args).output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            return Err(SyncError::GitFailed(format!(
                "args={args:?}\nstdout={stdout}\nstderr={stderr}"
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn git_in_cache(&self, args: &[&str]) -> Result<String, SyncError> {
        let cache_str = self.cache_dir.to_string_lossy().to_string();
        let mut full_args = vec!["-C", &cache_str];
        full_args.extend_from_slice(args);
        let output = Command::new("git").args(&full_args).output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            return Err(SyncError::GitFailed(format!(
                "args={full_args:?}\nstdout={stdout}\nstderr={stderr}"
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    pub fn pull(&self) -> Result<SyncResult, SyncError> {
        let now = Utc::now();

        if !self.cache_dir.exists() {
            // Clone
            self.git(&[
                "clone",
                &self.repo_url,
                &self.cache_dir.to_string_lossy(),
            ])?;
        } else {
            // Record HEAD before fetch so we can count new commits
            let pre_fetch_head = self.git_in_cache(&["rev-parse", "HEAD"])
                .unwrap_or_default();
            let pre_fetch_head = pre_fetch_head.trim().to_string();

            // Fetch + merge
            self.git_in_cache(&["fetch", "--all"])?;
            // Try ff-only; if it fails (e.g., no origin/main yet), tolerate
            let _ = self.git_in_cache(&["merge", "origin/main", "--ff-only"]);

            let commits_fetched = if pre_fetch_head.is_empty() {
                0
            } else {
                let count_str = self
                    .git_in_cache(&[
                        "rev-list",
                        &format!("{pre_fetch_head}..HEAD"),
                        "--count",
                    ])
                    .unwrap_or_default();
                count_str.trim().parse::<u32>().unwrap_or(0)
            };

            let mut state = self.read_state()?;
            state.last_pulled_at = Some(now.to_rfc3339());
            state.commits_fetched = commits_fetched;
            self.write_state(&state)?;

            return Ok(SyncResult {
                pulled_at: now,
                commits_fetched,
            });
        }

        let mut state = self.read_state()?;
        state.last_pulled_at = Some(now.to_rfc3339());
        state.commits_fetched = 0;
        self.write_state(&state)?;

        Ok(SyncResult {
            pulled_at: now,
            commits_fetched: 0,
        })
    }

    pub fn push_node_changes(&self, message: &str) -> Result<PushResult, SyncError> {
        let branch = format!("nodes/{}", self.hostname);
        let node_path = format!("nodes/{}/", self.hostname);

        // Checkout or create the branch
        self.git_in_cache(&["checkout", "-B", &branch])?;

        // Stage only files under nodes/<hostname>/
        self.git_in_cache(&["add", &node_path])?;

        // Count staged files
        let staged = self.git_in_cache(&["diff", "--cached", "--name-only"])?;
        let files_committed = if staged.is_empty() {
            0u32
        } else {
            staged.lines().count() as u32
        };

        if files_committed == 0 {
            return Ok(PushResult {
                pushed_at: Utc::now(),
                branch,
                files_committed: 0,
            });
        }

        self.git_in_cache(&["commit", "-m", message])?;
        self.git_in_cache(&["push", "origin", &branch, "--force-with-lease"])?;

        let now = Utc::now();
        let mut state = self.read_state()?;
        state.last_pushed_at = Some(now.to_rfc3339());
        self.write_state(&state)?;

        Ok(PushResult {
            pushed_at: now,
            branch,
            files_committed,
        })
    }

    pub fn status(&self) -> Result<SyncStatus, SyncError> {
        let state = self.read_state()?;

        let last_pulled_at = state
            .last_pulled_at
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        let last_pushed_at = state
            .last_pushed_at
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        let node_branch_dirty = if self.cache_dir.exists() {
            let node_path = format!("nodes/{}/", self.hostname);
            let out = self
                .git_in_cache(&["status", "--porcelain", &node_path])
                .unwrap_or_default();
            !out.is_empty()
        } else {
            false
        };

        let fleet_branch_ahead = if self.cache_dir.exists() {
            let out = self
                .git_in_cache(&["rev-list", "HEAD..origin/main", "--count"])
                .unwrap_or_default();
            out.parse::<u32>().unwrap_or(0)
        } else {
            0
        };

        Ok(SyncStatus {
            last_pulled_at,
            last_pushed_at,
            node_branch_dirty,
            fleet_branch_ahead,
        })
    }

    pub fn last_pulled_at(&self) -> Result<Option<DateTime<Utc>>, SyncError> {
        let state = self.read_state()?;
        Ok(state
            .last_pulled_at
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn setup_bare_and_cache(dir: &Path) -> (PathBuf, PathBuf) {
        let bare = dir.join("bare");
        let work = dir.join("work");
        let cache = dir.join("cache");

        // Init bare repo
        Command::new("git")
            .args(["init", "--bare", bare.to_str().unwrap()])
            .output()
            .unwrap();

        // Clone into work, make an initial commit, push
        Command::new("git")
            .args(["clone", bare.to_str().unwrap(), work.to_str().unwrap()])
            .output()
            .unwrap();

        std::fs::write(work.join("README"), "hello").unwrap();

        Command::new("git")
            .args(["-C", work.to_str().unwrap(), "add", "."])
            .output()
            .unwrap();

        Command::new("git")
            .args([
                "-C",
                work.to_str().unwrap(),
                "-c",
                "user.email=test@test.com",
                "-c",
                "user.name=Test",
                "commit",
                "-m",
                "init",
            ])
            .output()
            .unwrap();

        Command::new("git")
            .args(["-C", work.to_str().unwrap(), "push", "origin", "main"])
            .output()
            .unwrap();

        (bare, cache)
    }

    #[test]
    fn new_does_not_clone() {
        let dir = tempdir().unwrap();
        let cache = dir.path().join("sync");
        let client =
            SyncClient::new("https://example.com/fake.git", &cache, "testhost").unwrap();
        assert!(!cache.exists());
        drop(client);
    }

    #[test]
    fn pull_clones_on_first_call() {
        let dir = tempdir().unwrap();
        let (bare, cache) = setup_bare_and_cache(dir.path());

        let client = SyncClient::new(bare.to_str().unwrap(), &cache, "testhost").unwrap();
        let result = client.pull().unwrap();

        assert!(cache.exists());
        assert!(result.pulled_at <= Utc::now());
    }

    #[test]
    fn push_node_changes_commits_and_reports() {
        let dir = tempdir().unwrap();
        let (bare, cache) = setup_bare_and_cache(dir.path());

        // Clone into cache first (simulate prior pull)
        Command::new("git")
            .args(["clone", bare.to_str().unwrap(), cache.to_str().unwrap()])
            .output()
            .unwrap();

        // Configure git identity in cache clone
        Command::new("git")
            .args([
                "-C",
                cache.to_str().unwrap(),
                "config",
                "user.email",
                "test@test.com",
            ])
            .output()
            .unwrap();
        Command::new("git")
            .args([
                "-C",
                cache.to_str().unwrap(),
                "config",
                "user.name",
                "Test",
            ])
            .output()
            .unwrap();

        // Write a file under nodes/testhost/
        std::fs::create_dir_all(cache.join("nodes/testhost")).unwrap();
        std::fs::write(cache.join("nodes/testhost/config.yaml"), "key: value").unwrap();

        let client = SyncClient::new(bare.to_str().unwrap(), &cache, "testhost").unwrap();
        let result = client.push_node_changes("test push").unwrap();

        assert_eq!(result.files_committed, 1);
        assert!(result.branch.contains("testhost"));
    }
}

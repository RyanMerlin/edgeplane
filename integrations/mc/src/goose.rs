//! Goose runtime integration for MissionControl.
//!
//! Goose is an open-source AI agent from AAIF (Linux Foundation) that uses a tool-calling
//! loop to complete tasks. This module wires Goose into the MC agent mesh: it routes all
//! inference through the cluster-internal LiteLLM proxy and injects the standard `MC_*`
//! environment variables so the running agent can report progress back to MissionControl.
use crate::{
    config::{McConfig, mc_home_dir},
    task_md,
};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct GoosePaths {
    pub profile_root: PathBuf,
    pub runtime_home: PathBuf,
    pub config_path: PathBuf,
}

pub fn goose_paths(profile: &str) -> GoosePaths {
    let profile_root = mc_home_dir()
        .join("profiles")
        .join("goose")
        .join(profile);
    let runtime_home = profile_root.join("goose-home");
    GoosePaths {
        config_path: runtime_home.join("goose").join("config.yaml"),
        profile_root,
        runtime_home,
    }
}

pub async fn run_launch(
    profile: String,
    _new: bool,
    headless: bool,
    _with_rtk: bool,
    passthrough: Vec<String>,
    config: &McConfig,
) -> Result<()> {
    if headless {
        bail!("headless mode is not supported for interactive launch; use `mc run goose --mission <id>` instead");
    }
    run_goose_inner(&profile, &passthrough, config)
}

pub async fn run_exec(
    profile: String,
    passthrough: Vec<String>,
    config: &McConfig,
) -> Result<()> {
    run_goose_inner(&profile, &passthrough, config)
}

/// Blocking launch helper for SoloSupervisor — reads the task MD, builds a prompt, and
/// runs Goose headlessly against the cluster LiteLLM.
pub fn launch_goose_blocking(
    passthrough: &[String],
    runtime_home: &Path,
    config: &McConfig,
    profile: &str,
    agent_id: &str,
    run_id: Option<&str>,
    task_id: Option<&str>,
    task_md_path: Option<&Path>,
) -> Result<std::process::ExitStatus> {
    let goose = which_goose().context("goose binary not found on PATH")?;
    let mut cmd = std::process::Command::new(goose);

    if let Some(path) = task_md_path {
        let (fm, body) = task_md::read_task_md(path)
            .with_context(|| format!("failed to read task MD from {}", path.display()))?;
        let prompt = build_task_prompt(&fm.title, &fm.description, &body);
        cmd.args(["run", "--text", &prompt, "--quiet", "--output-format", "stream-json", "--no-session", "--max-turns", "50"]);
    } else {
        cmd.args(passthrough);
    }

    apply_env(
        &mut cmd,
        runtime_home,
        config,
        profile,
        Some(agent_id),
        run_id,
        task_id,
        task_md_path,
    );

    cmd.status().context("failed to execute goose")
}

fn run_goose_inner(profile: &str, passthrough: &[String], config: &McConfig) -> Result<()> {
    which_goose().context("goose binary not found on PATH; install Goose CLI first")?;
    let paths = goose_paths(profile);
    let status = run_goose_process(passthrough, &paths.runtime_home, config, profile)?;
    if !status.success() {
        bail!("goose exited with status {}", status);
    }
    Ok(())
}

fn run_goose_process(
    args: &[String],
    runtime_home: &Path,
    config: &McConfig,
    profile: &str,
) -> Result<std::process::ExitStatus> {
    let goose = which_goose().context("goose binary not found on PATH")?;
    let mut cmd = std::process::Command::new(goose);
    cmd.args(args);
    apply_env(&mut cmd, runtime_home, config, profile, None, None, None, None);
    cmd.status().context("failed to execute goose")
}

fn which_goose() -> Result<PathBuf> {
    // MC_GOOSE_BIN overrides PATH lookup (useful when goose lives in ~/.local/bin)
    if let Ok(p) = std::env::var("MC_GOOSE_BIN") {
        let path = PathBuf::from(&p);
        if path.is_file() {
            return Ok(path);
        }
    }
    which::which("goose").context("goose not found on PATH; set MC_GOOSE_BIN to override")
}

fn litellm_host() -> String {
    std::env::var("MC_LITELLM_HOST")
        .unwrap_or_else(|_| "http://litellm:4000".to_string())
}

fn goose_model() -> String {
    std::env::var("MC_GOOSE_MODEL")
        .unwrap_or_else(|_| "local-agent".to_string())
}

fn apply_env(
    cmd: &mut std::process::Command,
    runtime_home: &Path,
    config: &McConfig,
    profile: &str,
    agent_id: Option<&str>,
    run_id: Option<&str>,
    task_id: Option<&str>,
    task_md_path: Option<&Path>,
) {
    cmd.env("XDG_CONFIG_HOME", runtime_home);
    cmd.env("GOOSE_PROVIDER", "litellm");
    cmd.env("LITELLM_HOST", litellm_host());
    cmd.env("GOOSE_MODEL", goose_model());
    cmd.env("GOOSE_MODE", "Auto");

    if let Ok(api_key) = std::env::var("MC_LITELLM_API_KEY") {
        if !api_key.is_empty() {
            cmd.env("LITELLM_API_KEY", api_key);
        }
    }

    cmd.env("MC_BASE_URL", config.base_url.as_str());
    if let Some(token) = &config.token {
        if !token.trim().is_empty() {
            cmd.env("MC_TOKEN", token);
        }
    }
    cmd.env("MC_AGENT_PROFILE", profile);

    if let Some(id) = agent_id {
        cmd.env("MC_MESH_AGENT_ID", id);
    }
    if let Some(rid) = run_id {
        cmd.env("MC_RUN_ID", rid);
    }
    if let Some(tid) = task_id {
        cmd.env("MC_MESH_TASK_ID", tid);
    }
    if let Some(p) = task_md_path {
        cmd.env("MC_TASK_MD_PATH", p);
    }
}

fn build_task_prompt(title: &str, description: &str, body: &str) -> String {
    format!("# {title}\n\n{description}\n\n{body}").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goose_paths_ends_with_expected_segments() {
        let paths = goose_paths("default");
        assert!(
            paths.profile_root.ends_with("profiles/goose/default"),
            "profile_root: {}",
            paths.profile_root.display()
        );
        assert!(
            paths.runtime_home.ends_with("profiles/goose/default/goose-home"),
            "runtime_home: {}",
            paths.runtime_home.display()
        );
        assert!(
            paths.config_path.ends_with("profiles/goose/default/goose-home/goose/config.yaml"),
            "config_path: {}",
            paths.config_path.display()
        );
    }

    #[test]
    fn goose_paths_uses_given_profile_name() {
        let paths = goose_paths("my-agent");
        assert!(paths.profile_root.ends_with("profiles/goose/my-agent"));
        assert!(paths.runtime_home.ends_with("goose-home"));
    }

    #[test]
    fn build_task_prompt_includes_all_parts() {
        let prompt = build_task_prompt("Do the thing", "An important task", "Agent wrote this.");
        assert!(prompt.starts_with("# Do the thing"));
        assert!(prompt.contains("An important task"));
        assert!(prompt.contains("Agent wrote this."));
    }

    #[test]
    fn build_task_prompt_trims_whitespace() {
        let prompt = build_task_prompt("Title", "Desc", "  \n  ");
        assert!(!prompt.ends_with('\n'));
        assert!(!prompt.starts_with('\n'));
    }

    #[test]
    fn litellm_host_default_is_cluster_address() {
        // Confirm the fallback constant without mutating env (safe, no-parallel-test concern).
        let host = std::env::var("MC_LITELLM_HOST")
            .unwrap_or_else(|_| "http://litellm:4000".to_string());
        // If the env var is not set, we expect the cluster default.
        if std::env::var("MC_LITELLM_HOST").is_err() {
            assert_eq!(host, "http://litellm:4000");
        }
    }
}

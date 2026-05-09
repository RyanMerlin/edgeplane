use crate::{
    config::{McConfig, mc_home_dir},
    mc_info, mc_ok,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const MC_CODEX_MARKER_BEGIN: &str = "# mc-codex: missioncontrol managed block (begin)";
const MC_CODEX_MARKER_END: &str = "# mc-codex: missioncontrol managed block (end)";

#[derive(Debug, Clone)]
pub struct CodexPaths {
    pub profile_root: PathBuf,
    pub runtime_home: PathBuf,
    pub config_path: PathBuf,
    pub ownership_path: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
struct CodexDoctorIssue {
    code: String,
    severity: String,
    detail: String,
    #[serde(default)]
    fixable: bool,
}

#[derive(Debug, Clone, Serialize)]
struct CodexDoctorReport {
    tool: String,
    profile: String,
    auth_mode: String,
    ready: bool,
    fixable: bool,
    repaired: bool,
    status: String,
    issues: Vec<CodexDoctorIssue>,
    repaired_actions: Vec<String>,
    suggested_command: String,
}

#[derive(Debug, Clone, Serialize)]
struct CodexStatusReport {
    tool: String,
    profile: String,
    auth_mode: String,
    ready: bool,
    status: String,
    issues: Vec<CodexDoctorIssue>,
    suggested_command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OwnershipState {
    schema_version: u32,
    managed_files: Vec<String>,
    managed_entries: Vec<String>,
    last_repaired_at: Option<String>,
}

/// Launch Codex in a prepared profile runtime (auto-repair + resume).
pub async fn run_launch(
    profile: String,
    new: bool,
    _headless: bool,
    _with_rtk: bool,
    _passthrough: Vec<String>,
    config: &McConfig,
) -> Result<()> {
    let report = inspect_profile(&profile, config, true)?;

    if !report.ready {
        bail!(
            "{}: not ready; run `mc run codex doctor --fix -p {}`",
            profile,
            profile
        );
    }

    if report.repaired {
        mc_ok!("{}: repaired drift", profile);
    } else {
        mc_ok!("{}: ready", profile);
    }

    let paths = codex_paths(&profile);
    if new {
        mc_info!("{}: starting new session", profile);
        let status = run_codex_process(
            &build_new_session_args(),
            &paths.runtime_home,
            config,
            &profile,
        )?;
        if !status.success() {
            bail!("codex exited with status {}", status);
        }
        return Ok(());
    }

    mc_info!("{}: resuming", profile);
    let resume_status =
        run_codex_process(&build_resume_args(), &paths.runtime_home, config, &profile)?;
    if !resume_status.success() {
        mc_info!("{}: resume unavailable; starting new session", profile);
        let fresh_status = run_codex_process(
            &build_new_session_args(),
            &paths.runtime_home,
            config,
            &profile,
        )?;
        if !fresh_status.success() {
            bail!("codex exited with status {}", fresh_status);
        }
    }
    Ok(())
}

/// Inspect and optionally repair Codex runtime readiness.
pub async fn run_doctor(
    profile: String,
    fix: bool,
    json: bool,
    _headless: bool,
    config: &McConfig,
) -> Result<()> {
    let report = inspect_profile(&profile, config, fix)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return if report.ready {
            Ok(())
        } else {
            bail!("profile not ready")
        };
    }

    println!("Profile: {}", report.profile);
    println!(
        "Status: {}",
        if report.ready { "ready" } else { "not ready" }
    );
    if report.issues.is_empty() {
        println!("Issues: none");
    } else {
        println!("Issues:");
        for issue in &report.issues {
            println!("  - {} ({}) {}", issue.code, issue.severity, issue.detail);
        }
    }
    if !report.repaired_actions.is_empty() {
        println!("Repaired:");
        for action in &report.repaired_actions {
            println!("  - {}", action);
        }
    }
    if !report.ready {
        println!("Fix: {}", report.suggested_command);
    }

    if report.ready {
        Ok(())
    } else {
        bail!("profile not ready")
    }
}

/// Read-only runtime status (exits 0 even when not ready).
pub async fn run_status(profile: String, json: bool, config: &McConfig) -> Result<()> {
    let report = inspect_profile(&profile, config, false)?;
    let status = CodexStatusReport {
        tool: report.tool,
        profile: report.profile,
        auth_mode: report.auth_mode,
        ready: report.ready,
        status: report.status,
        issues: report.issues,
        suggested_command: report.suggested_command,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&status)?);
        return Ok(());
    }

    println!("Profile: {}", status.profile);
    println!(
        "Status: {}",
        if status.ready { "ready" } else { "not ready" }
    );
    println!("Auth: {}", status.auth_mode);
    if status.issues.is_empty() {
        println!("Issues: none");
    } else {
        println!("Issues:");
        for issue in &status.issues {
            println!("  - {} ({}) {}", issue.code, issue.severity, issue.detail);
        }
    }
    if !status.ready {
        println!("Fix: {}", status.suggested_command);
    }
    Ok(())
}

/// Thin native Codex execution — passthrough args verbatim to the codex binary.
pub async fn run_exec(
    profile: String,
    passthrough: Vec<String>,
    config: &McConfig,
) -> Result<()> {
    let paths = codex_paths(&profile);

    if which_binary("codex").is_err() {
        bail!("native codex binary not found on PATH; install Codex CLI first");
    }
    if !paths.runtime_home.exists() || !paths.config_path.exists() {
        bail!(
            "{}: runtime is not prepared; run `mc run codex doctor --fix -p {}`",
            profile,
            profile
        );
    }

    let forwarded = normalize_exec_args(passthrough);

    let status = run_codex_process(&forwarded, &paths.runtime_home, config, &profile)?;
    if !status.success() {
        bail!("codex exited with status {}", status);
    }
    Ok(())
}

pub fn codex_paths(profile: &str) -> CodexPaths {
    let profile_root = mc_home_dir().join("profiles").join("codex").join(profile);
    let runtime_home = profile_root.join("codex-home");
    CodexPaths {
        config_path: runtime_home.join("config.toml"),
        ownership_path: profile_root.join("meta").join("ownership.json"),
        profile_root,
        runtime_home,
    }
}

fn inspect_profile(profile: &str, config: &McConfig, repair: bool) -> Result<CodexDoctorReport> {
    let mut issues = Vec::<CodexDoctorIssue>::new();
    let mut repaired_actions = Vec::<String>::new();
    let mut repaired = false;
    let mut has_unfixable = false;

    let paths = codex_paths(profile);

    if which_binary("codex").is_err() {
        issues.push(issue(
            "NATIVE_CODEX_NOT_FOUND",
            "fatal",
            "native `codex` binary is not on PATH",
            false,
        ));
        has_unfixable = true;
    }

    if !paths.profile_root.exists() {
        issues.push(issue(
            "RUNTIME_HOME_MISSING",
            "error",
            "profile runtime root does not exist",
            true,
        ));
        if repair {
            fs::create_dir_all(paths.profile_root.join("meta"))?;
            fs::create_dir_all(&paths.runtime_home)?;
            repaired = true;
            repaired_actions.push("created profile runtime layout".to_string());
        }
    }

    if !paths.runtime_home.exists() {
        issues.push(issue(
            "RUNTIME_HOME_MISSING",
            "error",
            "CODEX_HOME directory missing",
            true,
        ));
        if repair {
            fs::create_dir_all(&paths.runtime_home)?;
            repaired = true;
            repaired_actions.push("created CODEX_HOME".to_string());
        }
    }

    if !paths.config_path.exists() {
        issues.push(issue(
            "MC_CONFIG_MISSING",
            "error",
            "config.toml missing",
            true,
        ));
        if repair {
            write_codex_config(&paths.config_path, config, None)?;
            repaired = true;
            repaired_actions.push("wrote minimal config.toml".to_string());
        }
    }

    if paths.config_path.exists() {
        let raw = fs::read_to_string(&paths.config_path).unwrap_or_default();
        if !raw.contains("[mcp_servers.missioncontrol]") {
            issues.push(issue(
                "MC_MCP_CONFIG_MISSING",
                "error",
                "Mission Control MCP entry missing from config.toml",
                true,
            ));
            if repair {
                write_codex_config(&paths.config_path, config, Some(&raw))?;
                repaired = true;
                repaired_actions.push("patched missioncontrol MCP entry".to_string());
            }
        } else {
            let desired_command = desired_mc_command();
            let expected = format!("command = \"{}\"", escape_toml(&desired_command));
            if !raw.contains(&expected) {
                issues.push(issue(
                    "MC_MCP_COMMAND_DRIFT",
                    "error",
                    "missioncontrol MCP command is not pinned to the installed mc binary",
                    true,
                ));
                if repair {
                    write_codex_config(&paths.config_path, config, Some(&raw))?;
                    repaired = true;
                    repaired_actions.push("repaired missioncontrol MCP command path".to_string());
                }
            }
        }
    }

    if repair && seed_minimal_codex_state(&paths)? {
        repaired = true;
        repaired_actions
            .push("seeded Codex auth and sandbox state from global ~/.codex".to_string());
    }

    // OAuth-first default: rely on native Codex login status instead of API-key checks.
    // This supports both file-backed and keyring-backed auth under CODEX_HOME.
    if !is_codex_login_available(&paths.runtime_home, profile)? {
        if repair
            && seed_minimal_codex_state(&paths)?
            && is_codex_login_available(&paths.runtime_home, profile)?
        {
            repaired = true;
            repaired_actions.push("seeded Codex auth from global ~/.codex".to_string());
        } else {
            issues.push(issue(
                "AUTH_STATE_MISSING",
                "error",
                "codex is not authenticated for this profile; run `CODEX_HOME=<profile-home> codex login`",
                false,
            ));
            has_unfixable = true;
        }
    }

    if repair && !has_unfixable {
        write_ownership(&paths)?;
    }

    if repair {
        let (refreshed_issues, _refreshed_unfixable) =
            collect_profile_issues(&paths, config, profile)?;
        issues = refreshed_issues;
    }

    let ready = !issues
        .iter()
        .any(|i| i.severity == "error" || i.severity == "fatal");
    let fixable = issues
        .iter()
        .filter(|i| i.severity == "error" || i.severity == "fatal")
        .all(|i| i.fixable);
    let status = if ready {
        "ok"
    } else if fixable {
        "repairable"
    } else {
        "blocked"
    }
    .to_string();

    Ok(CodexDoctorReport {
        tool: "codex".to_string(),
        profile: profile.to_string(),
        auth_mode: "chatgpt_native".to_string(),
        ready,
        fixable,
        repaired,
        status,
        issues,
        repaired_actions,
        suggested_command: format!("mc run codex doctor --fix -p {}", profile),
    })
}

fn collect_profile_issues(
    paths: &CodexPaths,
    _config: &McConfig,
    profile: &str,
) -> Result<(Vec<CodexDoctorIssue>, bool)> {
    let mut issues = Vec::<CodexDoctorIssue>::new();
    let mut has_unfixable = false;

    if which_binary("codex").is_err() {
        issues.push(issue(
            "NATIVE_CODEX_NOT_FOUND",
            "fatal",
            "native `codex` binary is not on PATH",
            false,
        ));
        has_unfixable = true;
    }

    if !paths.profile_root.exists() {
        issues.push(issue(
            "RUNTIME_HOME_MISSING",
            "error",
            "profile runtime root does not exist",
            true,
        ));
    }

    if !paths.runtime_home.exists() {
        issues.push(issue(
            "RUNTIME_HOME_MISSING",
            "error",
            "CODEX_HOME directory missing",
            true,
        ));
    }

    if !paths.config_path.exists() {
        issues.push(issue(
            "MC_CONFIG_MISSING",
            "error",
            "config.toml missing",
            true,
        ));
    } else {
        let raw = fs::read_to_string(&paths.config_path).unwrap_or_default();
        if !raw.contains("[mcp_servers.missioncontrol]") {
            issues.push(issue(
                "MC_MCP_CONFIG_MISSING",
                "error",
                "Mission Control MCP entry missing from config.toml",
                true,
            ));
        } else {
            let desired_command = desired_mc_command();
            let expected = format!("command = \"{}\"", escape_toml(&desired_command));
            if !raw.contains(&expected) {
                issues.push(issue(
                    "MC_MCP_COMMAND_DRIFT",
                    "error",
                    "missioncontrol MCP command is not pinned to the installed mc binary",
                    true,
                ));
            }
        }
    }

    if !is_codex_login_available(&paths.runtime_home, profile)? {
        issues.push(issue(
            "AUTH_STATE_MISSING",
            "error",
            "codex is not authenticated for this profile; run `CODEX_HOME=<profile-home> codex login`",
            false,
        ));
        has_unfixable = true;
    }

    Ok((issues, has_unfixable))
}

fn is_codex_login_available(runtime_home: &Path, profile: &str) -> Result<bool> {
    let mut cmd = resolved_command("codex");
    cmd.arg("login")
        .arg("status")
        .env("CODEX_HOME", runtime_home)
        .env("MC_AGENT_PROFILE", profile);
    let status = cmd
        .status()
        .context("failed to execute `codex login status`")?;
    Ok(status.success())
}

fn issue(code: &str, severity: &str, detail: &str, fixable: bool) -> CodexDoctorIssue {
    CodexDoctorIssue {
        code: code.to_string(),
        severity: severity.to_string(),
        detail: detail.to_string(),
        fixable,
    }
}

fn build_resume_args() -> Vec<String> {
    vec!["resume".to_string(), "--last".to_string()]
}

fn build_new_session_args() -> Vec<String> {
    Vec::new()
}

fn normalize_exec_args(mut args: Vec<String>) -> Vec<String> {
    if args.first().map(|v| v.as_str()) == Some("--") {
        args.remove(0);
    }
    args
}

fn write_ownership(paths: &CodexPaths) -> Result<()> {
    if let Some(parent) = paths.ownership_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let state = OwnershipState {
        schema_version: 1,
        managed_files: vec![
            rel_for_ownership(paths, &paths.config_path),
            rel_for_ownership(paths, &paths.ownership_path),
        ],
        managed_entries: vec!["mcp_servers.missioncontrol".to_string()],
        last_repaired_at: Some(chrono::Utc::now().to_rfc3339()),
    };
    fs::write(&paths.ownership_path, serde_json::to_string_pretty(&state)?)?;
    Ok(())
}

fn rel_for_ownership(paths: &CodexPaths, value: &Path) -> String {
    value
        .strip_prefix(&paths.profile_root)
        .map(|v| v.display().to_string())
        .unwrap_or_else(|_| value.display().to_string())
}

fn write_codex_config(path: &Path, config: &McConfig, existing: Option<&str>) -> Result<()> {
    let mut content = existing.unwrap_or_default().to_string();
    if !content.is_empty() {
        content = strip_mc_managed_block(&content);
        if !content.ends_with('\n') {
            content.push('\n');
        }
        content.push('\n');
    }

    content.push_str(&render_mc_managed_block(
        config.base_url.as_str().trim_end_matches('/'),
        config.token.as_deref(),
    ));

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

fn desired_mc_command() -> String {
    std::env::current_exe()
        .ok()
        .filter(|p| p.is_file())
        .or_else(|| which_binary("mc").ok())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "mc".to_string())
}

fn strip_mc_managed_block(existing: &str) -> String {
    let mut output = Vec::<String>::new();
    let mut skip = false;

    for line in existing.lines() {
        if line.trim() == MC_CODEX_MARKER_BEGIN {
            skip = true;
            continue;
        }
        if line.trim() == MC_CODEX_MARKER_END {
            skip = false;
            continue;
        }
        if !skip {
            output.push(line.to_string());
        }
    }

    output.join("\n").trim_end().to_string()
}

fn render_mc_managed_block(base_url: &str, token: Option<&str>) -> String {
    let mc_command = desired_mc_command();

    let mut buf = String::new();
    buf.push_str(MC_CODEX_MARKER_BEGIN);
    buf.push('\n');
    buf.push_str("approval_policy = \"on-request\"\n");
    buf.push_str("sandbox_mode = \"workspace-write\"\n\n");
    buf.push_str("[mcp_servers.missioncontrol]\n");
    buf.push_str(&format!("command = \"{}\"\n", escape_toml(&mc_command)));
    buf.push_str("args = [\"serve\"]\n");
    buf.push_str("startup_timeout_sec = 30\n");
    buf.push_str("tool_timeout_sec = 60\n");

    match token {
        Some(value) if !value.trim().is_empty() => {
            buf.push_str(&format!(
                "env = {{ MC_BASE_URL = \"{}\", MC_TOKEN = \"{}\" }}\n",
                escape_toml(base_url),
                escape_toml(value)
            ));
        }
        _ => {
            buf.push_str(&format!(
                "env = {{ MC_BASE_URL = \"{}\" }}\n",
                escape_toml(base_url)
            ));
        }
    }

    buf.push_str(MC_CODEX_MARKER_END);
    buf.push('\n');
    buf
}

fn run_codex_process(
    args: &[String],
    runtime_home: &Path,
    config: &McConfig,
    profile: &str,
) -> Result<std::process::ExitStatus> {
    let mut cmd = resolved_command("codex");
    cmd.args(args);
    cmd.env("CODEX_HOME", runtime_home);
    cmd.env("MC_AGENT_PROFILE", profile);
    cmd.env("MC_BASE_URL", config.base_url.as_str());
    if let Some(token) = &config.token {
        if !token.trim().is_empty() {
            cmd.env("MC_TOKEN", token);
        }
    }

    cmd.status().context("failed to execute codex")
}

fn which_binary(name: &str) -> Result<PathBuf> {
    which::which(name).context(format!("binary `{}` not found on PATH", name))
}

pub fn resolved_command(name: &str) -> std::process::Command {
    let binary = which_binary(name).unwrap_or_else(|_| PathBuf::from(name));
    std::process::Command::new(binary)
}

/// Blocking launch helper for SoloSupervisor — sets MC_MESH_AGENT_ID / MC_RUN_ID env vars.
pub fn launch_codex_blocking(
    args: &[String],
    runtime_home: &Path,
    config: &McConfig,
    profile: &str,
    agent_id: &str,
    run_id: Option<&str>,
    task_id: Option<&str>,
    task_md_path: Option<&Path>,
) -> Result<std::process::ExitStatus> {
    let mut cmd = resolved_command("codex");
    cmd.args(args);
    cmd.env("CODEX_HOME", runtime_home);
    cmd.env("MC_AGENT_PROFILE", profile);
    cmd.env("MC_BASE_URL", config.base_url.as_str());
    cmd.env("MC_MESH_AGENT_ID", agent_id);
    if let Some(rid) = run_id {
        cmd.env("MC_RUN_ID", rid);
    }
    if let Some(tid) = task_id {
        cmd.env("MC_MESH_TASK_ID", tid);
    }
    if let Some(p) = task_md_path {
        cmd.env("MC_TASK_MD_PATH", p);
    }
    if let Some(token) = &config.token {
        if !token.trim().is_empty() {
            cmd.env("MC_TOKEN", token);
        }
    }
    cmd.status().context("failed to execute codex")
}

fn seed_minimal_codex_state(paths: &CodexPaths) -> Result<bool> {
    let Some(global_home) = dirs::home_dir() else {
        return Ok(false);
    };
    let mut changed = false;

    let global_codex = global_home.join(".codex");
    let runtime_codex = &paths.runtime_home;
    fs::create_dir_all(runtime_codex)?;

    let src_auth = global_codex.join("auth.json");
    let dst_auth = runtime_codex.join("auth.json");
    if src_auth.exists() && !dst_auth.exists() {
        fs::copy(&src_auth, &dst_auth).with_context(|| {
            format!(
                "failed to seed codex auth from {} to {}",
                src_auth.display(),
                dst_auth.display()
            )
        })?;
        changed = true;
    }

    let workspace_root = std::env::current_dir().context("unable to locate current directory")?;
    let sandbox_dir = runtime_codex.join(".sandbox");
    fs::create_dir_all(&sandbox_dir)?;
    let setup_marker = sandbox_dir.join("setup_marker.json");
    let global_setup_marker = global_codex.join(".sandbox").join("setup_marker.json");
    let template: serde_json::Value = if global_setup_marker.exists() {
        serde_json::from_str(&fs::read_to_string(&global_setup_marker)?)
            .unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    let mut root = template.as_object().cloned().unwrap_or_default();
    root.insert(
        "version".to_string(),
        template
            .get("version")
            .cloned()
            .unwrap_or_else(|| serde_json::json!(5)),
    );
    root.insert(
        "offline_username".to_string(),
        template
            .get("offline_username")
            .cloned()
            .unwrap_or_else(|| serde_json::json!("CodexSandboxOffline")),
    );
    root.insert(
        "online_username".to_string(),
        template
            .get("online_username")
            .cloned()
            .unwrap_or_else(|| serde_json::json!("CodexSandboxOnline")),
    );
    root.insert(
        "created_at".to_string(),
        serde_json::json!(chrono::Utc::now().to_rfc3339()),
    );
    let root_str = workspace_root.display().to_string();
    root.insert(
        "read_roots".to_string(),
        serde_json::json!([root_str.clone()]),
    );
    root.insert("write_roots".to_string(), serde_json::json!([root_str]));

    let desired = serde_json::to_string_pretty(&serde_json::Value::Object(root))?;
    let current = fs::read_to_string(&setup_marker).unwrap_or_default();
    if current != desired {
        fs::write(&setup_marker, desired)?;
        changed = true;
    }

    let setup_error = sandbox_dir.join("setup_error.json");
    if setup_error.exists() {
        fs::remove_file(&setup_error)?;
        changed = true;
    }

    Ok(changed)
}

fn escape_toml(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_block_removes_mc_managed_section_only() {
        let input = r#"a = 1
# mc-codex: missioncontrol managed block (begin)
[mcp_servers.missioncontrol]
command = \"mc\"
# mc-codex: missioncontrol managed block (end)
b = 2
"#;
        let out = strip_mc_managed_block(input);
        assert!(out.contains("a = 1"));
        assert!(out.contains("b = 2"));
        assert!(!out.contains("mcp_servers.missioncontrol"));
    }

    #[test]
    fn managed_block_without_token_omits_mc_token() {
        let out = render_mc_managed_block("http://localhost:8008", None);
        assert!(out.contains("MC_BASE_URL"));
        assert!(!out.contains("MC_TOKEN"));
    }

    #[test]
    fn managed_block_with_token_contains_mc_token() {
        let out = render_mc_managed_block("http://localhost:8008", Some("abc123"));
        assert!(out.contains("MC_BASE_URL"));
        assert!(out.contains("MC_TOKEN"));
        assert!(out.contains("abc123"));
    }

    #[test]
    fn strip_block_without_markers_preserves_content() {
        let input = "approval_policy = \"on-request\"\n";
        let out = strip_mc_managed_block(input);
        assert_eq!(out, "approval_policy = \"on-request\"");
    }

    #[test]
    fn resume_mode_args_are_stable() {
        assert_eq!(
            build_resume_args(),
            vec!["resume".to_string(), "--last".to_string()]
        );
        assert!(build_new_session_args().is_empty());
    }

    #[test]
    fn normalize_exec_args_strips_passthrough_sentinel_only() {
        assert_eq!(
            normalize_exec_args(vec!["--".into(), "--help".into()]),
            vec!["--help".to_string()]
        );
        assert_eq!(
            normalize_exec_args(vec!["exec".into(), "--json".into()]),
            vec!["exec".to_string(), "--json".to_string()]
        );
    }

    #[test]
    fn doctor_json_contract_contains_expected_top_level_fields() {
        let report = CodexDoctorReport {
            tool: "codex".to_string(),
            profile: "default".to_string(),
            auth_mode: "chatgpt_native".to_string(),
            ready: false,
            fixable: true,
            repaired: false,
            status: "repairable".to_string(),
            issues: vec![CodexDoctorIssue {
                code: "MC_CONFIG_MISSING".to_string(),
                severity: "error".to_string(),
                detail: "config.toml missing".to_string(),
                fixable: true,
            }],
            repaired_actions: vec![],
            suggested_command: "mc run codex doctor --fix".to_string(),
        };

        let value = serde_json::to_value(report).expect("serialize");
        let obj = value.as_object().expect("json object");
        for key in [
            "tool",
            "profile",
            "auth_mode",
            "ready",
            "fixable",
            "repaired",
            "status",
            "issues",
            "repaired_actions",
            "suggested_command",
        ] {
            assert!(obj.contains_key(key), "missing key: {key}");
        }
    }

    #[test]
    fn write_codex_config_repairs_missing_managed_block_and_preserves_user_content() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_path = temp.path().join("config.toml");
        std::fs::write(&config_path, "model = \"gpt-5\"\n").expect("seed config");

        let cfg = McConfig::from_parts(
            "http://localhost:8008",
            Some("tok".to_string()),
            None,
            None,
            None,
            10,
            false,
            false,
            false,
            None,
        )
        .expect("config");

        let existing = std::fs::read_to_string(&config_path).expect("read existing");
        write_codex_config(&config_path, &cfg, Some(&existing)).expect("repair write");
        let repaired = std::fs::read_to_string(&config_path).expect("read repaired");
        assert!(repaired.contains("model = \"gpt-5\""));
        assert!(repaired.contains("[mcp_servers.missioncontrol]"));
        assert!(repaired.contains("MC_BASE_URL"));
    }

    #[test]
    fn status_json_contract_contains_expected_top_level_fields() {
        let report = CodexStatusReport {
            tool: "codex".to_string(),
            profile: "default".to_string(),
            auth_mode: "chatgpt_native".to_string(),
            ready: false,
            status: "blocked".to_string(),
            issues: vec![CodexDoctorIssue {
                code: "AUTH_STATE_MISSING".to_string(),
                severity: "error".to_string(),
                detail: "not logged in".to_string(),
                fixable: false,
            }],
            suggested_command: "mc run codex doctor --fix".to_string(),
        };
        let value = serde_json::to_value(report).expect("serialize");
        let obj = value.as_object().expect("json object");
        for key in [
            "tool",
            "profile",
            "auth_mode",
            "ready",
            "status",
            "issues",
            "suggested_command",
        ] {
            assert!(obj.contains_key(key), "missing key: {key}");
        }
    }
}

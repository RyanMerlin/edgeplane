use crate::{
    config::{McConfig, mc_home_dir},
    mc_info, mc_ok, mc_warn,
};
use anyhow::{Context, Result, anyhow, bail};
use clap::ValueEnum;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
#[cfg(unix)]
use std::os::unix::fs as unix_fs;
use std::path::{Path, PathBuf};

#[derive(ValueEnum, Clone, Debug)]
enum ClaudeHookEvent {
    SessionStart,
    PostToolUse,
    SessionEnd,
}

#[derive(Debug, Clone)]
pub struct ClaudePaths {
    pub runtime_home: PathBuf,
    pub manifest_path: PathBuf,
    pub state_path: PathBuf,
    pub claude_config_path: PathBuf,
    pub settings_path: PathBuf,
    pub hooks_dir: PathBuf,
    pub self_link_path: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
struct ClaudeDoctorIssue {
    code: String,
    severity: String,
    detail: String,
    #[serde(default)]
    fixable: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ClaudeDoctorReport {
    profile: String,
    ready: bool,
    fixable: bool,
    repaired: bool,
    issues: Vec<ClaudeDoctorIssue>,
    suggested_command: String,
}

/// Launch Claude in a prepared profile runtime (auto-repair + resume).
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
            "{}: not ready; run `mc run claude doctor --fix -p {}`",
            profile,
            profile
        );
    }

    if report.repaired {
        mc_ok!("{}: repaired drift", profile);
    } else {
        mc_ok!("{}: ready", profile);
    }

    let paths = claude_paths(&profile);
    let has_resume = load_state_session(&paths.state_path).is_some();
    let use_resume = !new && has_resume;

    if use_resume {
        mc_info!("{}: resuming", profile);
    } else {
        mc_info!("{}: starting new session", profile);
    }

    let mut launch_args = Vec::<String>::new();
    if use_resume {
        launch_args.push("--resume".to_string());
    }

    let status = run_claude_process(&launch_args, &paths.runtime_home, config, &profile)?;
    if !status.success() && use_resume {
        mc_warn!("{}: resume failed; retrying with new session", profile);
        let retry_status = run_claude_process(&[], &paths.runtime_home, config, &profile)?;
        if !retry_status.success() {
            bail!("claude exited with status {}", retry_status);
        }
    } else if !status.success() {
        bail!("claude exited with status {}", status);
    }

    Ok(())
}

/// Inspect and optionally repair Claude runtime readiness.
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
    if !report.ready {
        println!("Fix: {}", report.suggested_command);
    }

    if report.ready {
        Ok(())
    } else {
        bail!("profile not ready")
    }
}

/// Thin native Claude execution — passthrough args verbatim to the claude binary.
pub async fn run_exec(
    profile: String,
    passthrough: Vec<String>,
    config: &McConfig,
) -> Result<()> {
    let paths = claude_paths(&profile);

    if which_binary("claude").is_err() {
        bail!(
            "native claude binary not found on PATH; install with: npm install -g @anthropic-ai/claude-code"
        );
    }
    if !paths.runtime_home.exists() || !paths.claude_config_path.exists() {
        bail!(
            "{}: runtime is not prepared; run `mc run claude doctor --fix -p {}`",
            profile,
            profile
        );
    }

    let status = run_claude_process(&passthrough, &paths.runtime_home, config, &profile)?;
    if !status.success() {
        bail!("claude exited with status {}", status);
    }
    Ok(())
}

/// Internal lifecycle hook dispatcher — called by Claude hook scripts.
/// Invoked as: mc run claude hook --event <session-start|post-tool-use|session-end>
pub async fn run_hook(event: String, config: &McConfig) -> Result<()> {
    let hook_event = match event.as_str() {
        "session-start" => ClaudeHookEvent::SessionStart,
        "post-tool-use" => ClaudeHookEvent::PostToolUse,
        "session-end" => ClaudeHookEvent::SessionEnd,
        other => bail!("unknown hook event '{}'; valid: session-start, post-tool-use, session-end", other),
    };

    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let payload: Value = serde_json::from_str(&input).unwrap_or_else(|_| json!({}));

    if matches!(hook_event, ClaudeHookEvent::SessionStart) {
        if let Some(session_id) = payload.get("session_id").and_then(|v| v.as_str()) {
            let home = std::env::var("HOME").unwrap_or_default();
            let home_path = PathBuf::from(home);
            if let Some(runtime_root) = home_path.parent() {
                let state_path = runtime_root.join("state.json");
                let _ = write_state_session(&state_path, session_id);
            }
        }
    }

    let endpoint = match hook_event {
        ClaudeHookEvent::SessionStart => "/hooks/claude/session-start",
        ClaudeHookEvent::PostToolUse => "/hooks/claude/tool-audit",
        ClaudeHookEvent::SessionEnd => "/hooks/claude/session-end",
    };

    let base = config.base_url.as_str().trim_end_matches('/');
    let url = format!("{}{}", base, endpoint);
    let token = std::env::var("MC_AGENT_TOKEN")
        .ok()
        .or_else(|| std::env::var("MC_TOKEN").ok())
        .or_else(|| config.token.clone());

    let client = reqwest::Client::new();
    let mut req = client.post(url).json(&payload);
    if let Some(tok) = token {
        if !tok.trim().is_empty() {
            req = req.bearer_auth(tok);
        }
    }

    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status.is_success() {
                if matches!(hook_event, ClaudeHookEvent::SessionStart) && !body.trim().is_empty() {
                    print!("{}", body);
                }
                Ok(())
            } else {
                mc_warn!("claude hook {} returned HTTP {}", endpoint, status);
                Ok(())
            }
        }
        Err(err) => {
            mc_warn!("claude hook {} failed: {}", endpoint, err);
            Ok(())
        }
    }
}

pub fn claude_paths(profile: &str) -> ClaudePaths {
    let runtime_root = mc_home_dir()
        .join("profiles")
        .join(profile)
        .join("claude")
        .join("runtime");
    let runtime_home = runtime_root.join("home");
    ClaudePaths {
        manifest_path: runtime_root.join("manifest.json"),
        state_path: runtime_root.join("state.json"),
        claude_config_path: runtime_home.join(".claude.json"),
        settings_path: runtime_home.join(".claude").join("settings.json"),
        hooks_dir: runtime_home.join(".claude").join("hooks"),
        self_link_path: runtime_home.join(".local").join("bin").join("claude"),
        runtime_home,
    }
}

fn add_rtk_issues(issues: &mut Vec<ClaudeDoctorIssue>, paths: &ClaudePaths) {
    if which::which("rtk").is_ok() {
        if !check_rtk_hooks_configured(paths) {
            issues.push(issue(
                "RTK_NOT_CONFIGURED",
                "warning",
                "RTK is installed but hooks are not configured for this Claude profile. \
                 Run with --fix to install them, or run `rtk init` manually.",
                true,
            ));
        }
    } else {
        issues.push(issue(
            "RTK_NOT_INSTALLED",
            "info",
            "RTK not found in PATH. Install it for 60-90% token savings on agent runs. \
             See: brew install rtk",
            false,
        ));
    }
}

fn inspect_profile(profile: &str, config: &McConfig, fix: bool) -> Result<ClaudeDoctorReport> {
    let mut issues = Vec::<ClaudeDoctorIssue>::new();
    let mut repaired = false;
    let paths = claude_paths(profile);

    let claude_bin = match which_binary("claude") {
        Ok(path) => Some(path),
        Err(_) => {
            issues.push(issue(
                "NATIVE_CLAUDE_NOT_FOUND",
                "fatal",
                "claude binary not on PATH",
                false,
            ));
            None
        }
    };

    if !paths.runtime_home.exists() {
        issues.push(issue(
            "RUNTIME_HOME_MISSING",
            "error",
            "runtime home does not exist",
            true,
        ));
    }

    if !paths.claude_config_path.exists() {
        issues.push(issue(
            "MC_MCP_CONFIG_MISSING",
            "error",
            ".claude.json missing in runtime home",
            true,
        ));
    }

    if !paths.settings_path.exists() {
        issues.push(issue(
            "MC_HOOKS_MISSING",
            "error",
            "settings.json missing in runtime home",
            true,
        ));
    }

    if !paths.self_link_path.exists() {
        issues.push(issue(
            "RUNTIME_SELF_LINK_MISSING",
            "error",
            "runtime .local/bin/claude self-link missing",
            true,
        ));
    }

    add_rtk_issues(&mut issues, &paths);

    if fix {
        repaired = apply_repairs(&paths, config, claude_bin.as_deref())?;
        issues.clear();
        if which_binary("claude").is_err() {
            issues.push(issue(
                "NATIVE_CLAUDE_NOT_FOUND",
                "fatal",
                "claude binary not on PATH",
                false,
            ));
        }
        if !paths.runtime_home.exists() {
            issues.push(issue(
                "RUNTIME_HOME_MISSING",
                "error",
                "runtime home does not exist",
                true,
            ));
        }
        if !paths.claude_config_path.exists() {
            issues.push(issue(
                "MC_MCP_CONFIG_MISSING",
                "error",
                ".claude.json missing in runtime home",
                true,
            ));
        }
        if !paths.settings_path.exists() {
            issues.push(issue(
                "MC_HOOKS_MISSING",
                "error",
                "settings.json missing in runtime home",
                true,
            ));
        }
        if !paths.self_link_path.exists() {
            issues.push(issue(
                "RUNTIME_SELF_LINK_MISSING",
                "error",
                "runtime .local/bin/claude self-link missing",
                true,
            ));
        }

        add_rtk_issues(&mut issues, &paths);
    }

    let ready = issues
        .iter()
        .all(|i| i.severity != "error" && i.severity != "fatal");
    let fixable = issues
        .iter()
        .filter(|i| i.severity == "error" || i.severity == "fatal")
        .all(|i| i.fixable);

    Ok(ClaudeDoctorReport {
        profile: profile.to_string(),
        ready,
        fixable,
        repaired,
        issues,
        suggested_command: format!("mc run claude doctor --fix -p {}", profile),
    })
}

fn apply_repairs(
    paths: &ClaudePaths,
    config: &McConfig,
    claude_bin: Option<&Path>,
) -> Result<bool> {
    let mut changed = false;

    fs::create_dir_all(&paths.runtime_home)?;
    fs::create_dir_all(&paths.hooks_dir)?;

    changed |= seed_minimal_claude_state(paths)?;
    changed |= patch_mcp_config(&paths.claude_config_path, config)?;
    changed |= patch_hooks_config(&paths.settings_path)?;
    changed |= write_hook_wrappers(&paths.hooks_dir)?;

    if let Some(bin) = claude_bin {
        changed |= ensure_self_link(&paths.self_link_path, bin)?;
    }

    changed |= write_manifest(paths)?;

    // Repair RTK hooks if rtk is installed but not configured
    if which::which("rtk").is_ok() && !check_rtk_hooks_configured(paths) {
        match std::process::Command::new("rtk").arg("init").output() {
            Ok(out) if out.status.success() => {
                changed = true;
                tracing::info!("RTK hooks installed via `rtk init`");
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                tracing::warn!("rtk init failed: {}", stderr.trim());
            }
            Err(e) => {
                tracing::warn!("Failed to run rtk init: {}", e);
            }
        }
    }

    Ok(changed)
}

fn seed_minimal_claude_state(paths: &ClaudePaths) -> Result<bool> {
    let Some(global_home) = dirs::home_dir() else {
        return Ok(false);
    };
    let mut changed = false;

    for rel in [".claude/.credentials.json", ".claude/settings.json"] {
        let src = global_home.join(rel);
        let dst = paths.runtime_home.join(rel);
        if src.exists() && !dst.exists() {
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&src, &dst).with_context(|| {
                format!(
                    "failed to seed minimal claude state from {} to {}",
                    src.display(),
                    dst.display()
                )
            })?;
            changed = true;
        }
    }

    Ok(changed)
}

fn patch_mcp_config(config_path: &Path, config: &McConfig) -> Result<bool> {
    let mut root: Value = if config_path.exists() {
        serde_json::from_str(&fs::read_to_string(config_path)?)
            .unwrap_or_else(|_| Value::Object(Default::default()))
    } else {
        Value::Object(Default::default())
    };

    let old = serde_json::to_string(&root)?;

    let mcp_servers = root
        .as_object_mut()
        .ok_or_else(|| anyhow!("{} is not a JSON object", config_path.display()))?
        .entry("mcpServers")
        .or_insert_with(|| Value::Object(Default::default()))
        .as_object_mut()
        .ok_or_else(|| anyhow!("{} mcpServers is not an object", config_path.display()))?;

    let mc_command = std::env::current_exe()
        .ok()
        .filter(|p| p.is_file())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "mc".to_string());

    mcp_servers.insert(
        "missioncontrol".to_string(),
        json!({
            "command": mc_command,
            "args": ["serve"],
            "env": {
                "MC_BASE_URL": config.base_url.as_str().trim_end_matches('/')
            }
        }),
    );

    // Explicitly keep channel MCP opt-in only: remove managed experimental entry by default.
    mcp_servers.remove("missioncontrol_channel");

    let new = serde_json::to_string_pretty(&root)?;
    if old != serde_json::to_string(&root)? {
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(config_path, new)?;
        return Ok(true);
    }
    Ok(false)
}

fn patch_hooks_config(settings_path: &Path) -> Result<bool> {
    let mut root: Value = if settings_path.exists() {
        serde_json::from_str(&fs::read_to_string(settings_path)?)
            .unwrap_or_else(|_| Value::Object(Default::default()))
    } else {
        Value::Object(Default::default())
    };
    let before = serde_json::to_string(&root)?;

    let hooks_obj = root
        .as_object_mut()
        .ok_or_else(|| anyhow!("{} is not a JSON object", settings_path.display()))?
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow!("{} hooks is not an object", settings_path.display()))?;

    let specs = vec![
        (
            "SessionStart",
            json!({
                "matcher": "startup|resume",
                "hooks": [{"type":"command", "command":"\"${HOME}\"/.claude/hooks/mc-session-start.sh"}]
            }),
        ),
        (
            "PostToolUse",
            json!({
                "matcher": "mcp__missioncontrol__.*",
                "hooks": [{"type":"command", "command":"\"${HOME}\"/.claude/hooks/mc-post-tool-use.sh"}]
            }),
        ),
        (
            "SessionEnd",
            json!({
                "hooks": [{"type":"command", "command":"\"${HOME}\"/.claude/hooks/mc-session-end.sh"}]
            }),
        ),
    ];

    for (event, managed_entry) in specs {
        let arr = hooks_obj
            .entry(event.to_string())
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or_else(|| {
                anyhow!(
                    "{} hook event {} is not an array",
                    settings_path.display(),
                    event
                )
            })?;

        arr.retain(|entry| !is_managed_hook(entry));
        arr.push(managed_entry);
    }

    let after = serde_json::to_string(&root)?;
    if before != after {
        if let Some(parent) = settings_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(settings_path, serde_json::to_string_pretty(&root)?)?;
        return Ok(true);
    }
    Ok(false)
}

fn is_managed_hook(entry: &Value) -> bool {
    entry
        .get("hooks")
        .and_then(|v| v.as_array())
        .map(|hooks| {
            hooks.iter().any(|hook| {
                hook.get("command")
                    .and_then(|v| v.as_str())
                    .map(|cmd| cmd.contains("/.claude/hooks/mc-") || cmd.contains("/hooks/claude/"))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn write_hook_wrappers(hooks_dir: &Path) -> Result<bool> {
    fs::create_dir_all(hooks_dir)?;
    let mc_bin = std::env::current_exe()
        .ok()
        .filter(|p| p.is_file())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "mc".to_string());

    let scripts = [
        ("mc-session-start.sh", "session-start"),
        ("mc-post-tool-use.sh", "post-tool-use"),
        ("mc-session-end.sh", "session-end"),
    ];

    let mut changed = false;
    for (name, event) in scripts {
        let path = hooks_dir.join(name);
        let body = format!(
            "#!/usr/bin/env sh\nset -eu\nexec \"{}\" claude hook {}\n",
            mc_bin, event
        );
        let current = fs::read_to_string(&path).unwrap_or_default();
        if current != body {
            fs::write(&path, body)?;
            changed = true;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&path)?.permissions();
            if perms.mode() & 0o111 == 0 {
                perms.set_mode(0o755);
                fs::set_permissions(&path, perms)?;
                changed = true;
            }
        }
    }

    Ok(changed)
}

fn ensure_self_link(target: &Path, source: &Path) -> Result<bool> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }

    if target.exists() {
        let existing = fs::canonicalize(target).ok();
        let want = fs::canonicalize(source).ok();
        if existing.is_some() && want.is_some() && existing == want {
            return Ok(false);
        }
        let meta = fs::symlink_metadata(target)?;
        if meta.file_type().is_symlink() || meta.is_file() {
            fs::remove_file(target)?;
        }
    }

    #[cfg(unix)]
    {
        unix_fs::symlink(source, target)?;
    }
    #[cfg(not(unix))]
    {
        fs::copy(source, target)?;
    }
    Ok(true)
}

fn write_manifest(paths: &ClaudePaths) -> Result<bool> {
    let files = [
        &paths.claude_config_path,
        &paths.settings_path,
        &paths.hooks_dir.join("mc-session-start.sh"),
        &paths.hooks_dir.join("mc-post-tool-use.sh"),
        &paths.hooks_dir.join("mc-session-end.sh"),
        &paths.self_link_path,
    ];

    let entries = files
        .iter()
        .map(|path| {
            let hash = file_hash(path).unwrap_or_default();
            json!({
                "path": path.display().to_string(),
                "hash": hash,
                "ownership": "mc-managed"
            })
        })
        .collect::<Vec<_>>();

    let doc = json!({
        "schema_version": 1,
        "updated_at": chrono::Utc::now().to_rfc3339(),
        "managed": entries
    });

    let current = fs::read_to_string(&paths.manifest_path).unwrap_or_default();
    let body = serde_json::to_string_pretty(&doc)?;
    if current != body {
        if let Some(parent) = paths.manifest_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&paths.manifest_path, body)?;
        return Ok(true);
    }
    Ok(false)
}

fn file_hash(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let out = hasher.finalize();
    Some(hex::encode(out))
}

fn load_state_session(state_path: &Path) -> Option<String> {
    if !state_path.exists() {
        return None;
    }
    let root: Value = serde_json::from_str(&fs::read_to_string(state_path).ok()?).ok()?;
    root.get("last_session_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn write_state_session(state_path: &Path, session_id: &str) -> Result<()> {
    let mut root: Value = if state_path.exists() {
        serde_json::from_str(&fs::read_to_string(state_path)?)
            .unwrap_or_else(|_| Value::Object(Default::default()))
    } else {
        Value::Object(Default::default())
    };
    root.as_object_mut()
        .ok_or_else(|| anyhow!("state file is not JSON object"))?
        .insert(
            "last_session_id".to_string(),
            Value::String(session_id.to_string()),
        );
    root.as_object_mut().unwrap().insert(
        "updated_at".to_string(),
        Value::String(chrono::Utc::now().to_rfc3339()),
    );

    if let Some(parent) = state_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(state_path, serde_json::to_string_pretty(&root)?)?;
    Ok(())
}

fn run_claude_process(
    extra_args: &[String],
    runtime_home: &Path,
    config: &McConfig,
    profile: &str,
) -> Result<std::process::ExitStatus> {
    let mut cmd = resolved_command("claude");
    cmd.args(extra_args);
    cmd.env("HOME", runtime_home);
    cmd.env("MC_AGENT_PROFILE", profile);

    if let Some(token) = &config.token {
        if !token.trim().is_empty() {
            cmd.env("MC_TOKEN", token);
            cmd.env("MC_AGENT_TOKEN", token);
        }
    }

    let runtime_local_bin = runtime_home.join(".local").join("bin");
    if let Some(current_path) = std::env::var_os("PATH") {
        let new_path = std::env::join_paths(
            std::iter::once(runtime_local_bin).chain(std::env::split_paths(&current_path)),
        )
        .unwrap_or(current_path);
        cmd.env("PATH", new_path);
    }

    cmd.status().context("failed to spawn claude")
}

fn which_binary(name: &str) -> Result<PathBuf> {
    which::which(name).map_err(|_| anyhow!("not found on PATH"))
}

pub fn resolved_command(name: &str) -> std::process::Command {
    let binary = which_binary(name).unwrap_or_else(|_| PathBuf::from(name));
    std::process::Command::new(binary)
}

/// Blocking launch helper for SoloSupervisor — sets MC_MESH_AGENT_ID / MC_RUN_ID env vars.
pub fn launch_claude_blocking(
    extra_args: &[String],
    runtime_home: &Path,
    config: &McConfig,
    profile: &str,
    agent_id: &str,
    run_id: Option<&str>,
    task_id: Option<&str>,
    task_md_path: Option<&Path>,
) -> Result<std::process::ExitStatus> {
    let mut cmd = resolved_command("claude");
    cmd.args(extra_args);
    cmd.env("HOME", runtime_home);
    cmd.env("MC_AGENT_PROFILE", profile);
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
            cmd.env("MC_AGENT_TOKEN", token);
        }
    }
    let runtime_local_bin = runtime_home.join(".local").join("bin");
    if let Some(current_path) = std::env::var_os("PATH") {
        let new_path = std::env::join_paths(
            std::iter::once(runtime_local_bin).chain(std::env::split_paths(&current_path)),
        )
        .unwrap_or(current_path);
        cmd.env("PATH", new_path);
    }
    cmd.status().context("failed to spawn claude")
}

fn check_rtk_hooks_configured(paths: &ClaudePaths) -> bool {
    let hooks_dir = &paths.hooks_dir;
    if !hooks_dir.exists() {
        return false;
    }
    std::fs::read_dir(hooks_dir)
        .ok()
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .any(|e| {
                    let file_name = e.file_name();
                    let name_str = file_name.to_string_lossy();
                    // Check for RTK-specific hook filenames created by `rtk init`
                    name_str == "rtk-rewrite.sh" || name_str == ".rtk-hook.sha256"
                })
        })
        .unwrap_or(false)
}

fn issue(code: &str, severity: &str, detail: &str, fixable: bool) -> ClaudeDoctorIssue {
    ClaudeDoctorIssue {
        code: code.to_string(),
        severity: severity.to_string(),
        detail: detail.to_string(),
        fixable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_hook_detection_matches_mc_wrappers() {
        let v = json!({
            "hooks": [{"type":"command", "command":"\"${HOME}\"/.claude/hooks/mc-session-start.sh"}]
        });
        assert!(is_managed_hook(&v));
    }
}

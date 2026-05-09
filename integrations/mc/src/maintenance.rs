use crate::{client::MissionControlClient, config::McConfig};
use anyhow::Result;
use clap::{Args, ValueEnum};
use reqwest::Method;
use serde_json::{Value, json};
use std::fmt;
use std::time::Duration;
use tokio::time::timeout;
use uuid::Uuid;

#[derive(Args, Debug)]
pub struct DoctorArgs {
    #[arg(long, default_value = "/events/stream")]
    pub matrix_endpoint: String,
    #[arg(long, default_value_t = 5)]
    pub matrix_sample_seconds: u64,
    #[arg(long = "fix", default_value_t = false)]
    pub fix: bool,
    /// Also cleanup local profile/session artifacts after checks.
    #[arg(long, default_value_t = false)]
    pub cleanup: bool,
    /// When --cleanup is set, keep at most this many runtime instance dirs.
    #[arg(long, default_value_t = 8)]
    pub cleanup_keep_instances: usize,
    /// When --cleanup is set, keep at most this many bundle tar files per profile.
    #[arg(long, default_value_t = 6)]
    pub cleanup_keep_bundles: usize,
    /// When --cleanup is set, remove instance dirs older than this many days.
    #[arg(long, default_value_t = 7)]
    pub cleanup_max_age_days: u64,
}

#[derive(Args, Debug)]
pub struct BackupArgs {
    #[arg(long, value_enum, default_value = "all")]
    pub target: BackupTarget,
    #[arg(long)]
    pub reason: Option<String>,
}

#[derive(Args, Debug)]
pub struct ProfileGcArgs {
    /// Keep at most this many runtime instance dirs (newest first).
    #[arg(long, default_value_t = 20)]
    pub keep_instances: usize,
    /// Keep at most this many bundle tar files per profile (newest first).
    #[arg(long, default_value_t = 10)]
    pub keep_bundles: usize,
    /// Remove instance dirs older than this many days regardless of count.
    #[arg(long, default_value_t = 14)]
    pub max_age_days: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ProfileGcSummary {
    root: String,
    removed_instances: Vec<String>,
    removed_bundles: Vec<String>,
    keep_instances: usize,
    keep_bundles: usize,
    max_age_days: u64,
}

#[derive(ValueEnum, Clone, Debug)]
pub enum BackupTarget {
    Postgres,
    Rustfs,
    All,
}

impl fmt::Display for BackupTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            BackupTarget::Postgres => "postgres",
            BackupTarget::Rustfs => "rustfs",
            BackupTarget::All => "all",
        };
        f.write_str(label)
    }
}

pub async fn run_doctor_command(
    client: &MissionControlClient,
    config: &McConfig,
    args: &DoctorArgs,
) -> Result<()> {
    run_doctor(client, config, args).await
}

pub async fn run_backup_command(client: &MissionControlClient, args: BackupArgs) -> Result<()> {
    run_backup(client, args).await
}

pub fn run_profile_gc_command(config: &McConfig, args: ProfileGcArgs) -> Result<()> {
    run_profile_gc(config, args)
}

fn run_profile_gc(config: &McConfig, args: ProfileGcArgs) -> Result<()> {
    let summary = perform_profile_gc(args)?;
    print_json(&json!({
        "ok": true,
        "root": summary.root,
        "removed_instances": summary.removed_instances,
        "removed_bundles": summary.removed_bundles,
        "keep_instances": summary.keep_instances,
        "keep_bundles": summary.keep_bundles,
        "max_age_days": summary.max_age_days
    }));
    crate::mc_ok!(
        "profile-gc complete: removed {} instance dirs and {} bundle files",
        summary.removed_instances.len(),
        summary.removed_bundles.len()
    );
    let _ = config;
    Ok(())
}

fn perform_profile_gc(args: ProfileGcArgs) -> Result<ProfileGcSummary> {
    let root = crate::config::mc_home_dir();
    let mut removed_instances = Vec::<String>::new();
    let mut removed_bundles = Vec::<String>::new();

    let instances_dir = root.join("instances");
    if instances_dir.exists() {
        let mut entries: Vec<_> = std::fs::read_dir(&instances_dir)?
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().is_dir())
            .collect();
        entries.sort_by_key(|entry| {
            entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        });
        entries.reverse();

        let age_limit =
            std::time::Duration::from_secs(args.max_age_days.saturating_mul(24 * 60 * 60));
        let now = std::time::SystemTime::now();
        for (idx, entry) in entries.iter().enumerate() {
            let path = entry.path();
            let modified = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            let old = now.duration_since(modified).unwrap_or_default() > age_limit;
            if idx >= args.keep_instances || old {
                std::fs::remove_dir_all(&path)?;
                removed_instances.push(path.display().to_string());
            }
        }
    }

    let profiles_dir = root.join("profiles");
    if profiles_dir.exists() {
        for profile in std::fs::read_dir(&profiles_dir)?.filter_map(|entry| entry.ok()) {
            let bundles_dir = profile.path().join("bundles");
            if !bundles_dir.exists() {
                continue;
            }
            let mut bundles: Vec<_> = std::fs::read_dir(&bundles_dir)?
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry.path().is_file())
                .collect();
            bundles.sort_by_key(|entry| {
                entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
            });
            bundles.reverse();
            for (idx, bundle) in bundles.iter().enumerate() {
                if idx >= args.keep_bundles {
                    let path = bundle.path();
                    std::fs::remove_file(&path)?;
                    removed_bundles.push(path.display().to_string());
                }
            }
        }
    }

    Ok(ProfileGcSummary {
        root: root.display().to_string(),
        removed_instances,
        removed_bundles,
        keep_instances: args.keep_instances,
        keep_bundles: args.keep_bundles,
        max_age_days: args.max_age_days,
    })
}

async fn run_doctor(
    client: &MissionControlClient,
    config: &McConfig,
    args: &DoctorArgs,
) -> Result<()> {
    let checks = vec![
        run_health_check(client).await,
        run_tools_check(client).await,
        run_matrix_check(
            client,
            &args.matrix_endpoint,
            Duration::from_secs(args.matrix_sample_seconds),
        )
        .await,
        run_tailscale_check().await,
    ];
    let repairs = if args.fix {
        perform_repairs(config)
    } else {
        Vec::new()
    };
    let cleanup = if args.cleanup {
        let gc = perform_profile_gc(ProfileGcArgs {
            keep_instances: args.cleanup_keep_instances,
            keep_bundles: args.cleanup_keep_bundles,
            max_age_days: args.cleanup_max_age_days,
        })?;
        crate::mc_ok!(
            "doctor cleanup complete: removed {} instance dirs and {} bundle files",
            gc.removed_instances.len(),
            gc.removed_bundles.len()
        );
        Some(gc)
    } else {
        None
    };
    let report = DoctorReport {
        base_url: config.base_url.to_string(),
        agent_id: config.agent_context.agent_id.clone(),
        matrix_endpoint: args.matrix_endpoint.clone(),
        checks,
        repairs,
        cleanup,
    };
    println!(
        "Doctor report ({} checks, {} repairs)",
        report.checks.len(),
        report.repairs.len()
    );
    print_json(&serde_json::to_value(&report)?);
    Ok(())
}

async fn run_backup(client: &MissionControlClient, args: BackupArgs) -> Result<()> {
    let payload = json!({
        "target": args.target.to_string(),
        "reason": args.reason,
    });
    let response = client.post_json("/ops/backups", &payload).await?;
    print_json(&response);
    Ok(())
}

async fn run_health_check(client: &MissionControlClient) -> DoctorCheck {
    let start = std::time::Instant::now();
    let name = "mcp_health".to_string();
    match client.get_json("/mcp/health").await {
        Ok(payload) => DoctorCheck {
            name,
            ok: true,
            detail: "mcp health OK".into(),
            duration_ms: start.elapsed().as_millis(),
            payload: Some(payload),
            repair_hint: None,
        },
        Err(err) => DoctorCheck {
            name,
            ok: false,
            detail: err.to_string(),
            duration_ms: start.elapsed().as_millis(),
            payload: None,
            repair_hint: Some("Check MC_BASE_URL/MCP_TOKEN or OIDC configuration".into()),
        },
    }
}

async fn run_tools_check(client: &MissionControlClient) -> DoctorCheck {
    let start = std::time::Instant::now();
    let name = "mcp_tools".to_string();
    match client.get_json("/mcp/tools").await {
        Ok(payload) => DoctorCheck {
            name,
            ok: true,
            detail: "tools list succeeded".into(),
            duration_ms: start.elapsed().as_millis(),
            payload: Some(payload),
            repair_hint: None,
        },
        Err(err) => DoctorCheck {
            name,
            ok: false,
            detail: err.to_string(),
            duration_ms: start.elapsed().as_millis(),
            payload: None,
            repair_hint: Some("Ensure approvals/tools access and tokens are valid".into()),
        },
    }
}

async fn run_matrix_check(
    client: &MissionControlClient,
    endpoint: &str,
    sample_duration: Duration,
) -> DoctorCheck {
    let start = std::time::Instant::now();
    let name = "matrix_stream".to_string();
    let builder = match client.request_builder(Method::GET, endpoint) {
        Ok(builder) => builder,
        Err(err) => {
            return DoctorCheck {
                name,
                ok: false,
                detail: err.to_string(),
                duration_ms: start.elapsed().as_millis(),
                payload: None,
                repair_hint: Some("Invalid matrix endpoint; update --matrix-endpoint".into()),
            };
        }
    };
    let response = match timeout(
        sample_duration,
        builder
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .send(),
    )
    .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(err)) => {
            let hint = if err.to_string().to_lowercase().contains("tls") {
                Some("Run with MC_ALLOW_INSECURE=true for self-signed certs".into())
            } else {
                Some("Ensure /events/stream is reachable and not throttled".into())
            };
            return DoctorCheck {
                name,
                ok: false,
                detail: err.to_string(),
                duration_ms: start.elapsed().as_millis(),
                payload: None,
                repair_hint: hint,
            };
        }
        Err(_) => {
            return DoctorCheck {
                name,
                ok: false,
                detail: "matrix endpoint timed out".into(),
                duration_ms: start.elapsed().as_millis(),
                payload: None,
                repair_hint: Some("Verify the server is reachable and emitting events".into()),
            };
        }
    };
    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let ok = status.is_success() && content_type.contains("event-stream");
    let detail = if ok {
        "matrix endpoint streaming".into()
    } else {
        format!(
            "matrix endpoint returned {} with content-type {}",
            status, content_type
        )
    };
    let payload = Some(json!({
        "status": status.as_u16(),
        "content_type": content_type,
    }));
    drop(response);
    DoctorCheck {
        name,
        ok,
        detail,
        duration_ms: start.elapsed().as_millis(),
        payload,
        repair_hint: if ok {
            None
        } else {
            Some("Confirm the matrix listener is enabled and not blocked by firewalls".into())
        },
    }
}

async fn run_tailscale_check() -> DoctorCheck {
    let start = std::time::Instant::now();
    let mut check = tokio::task::spawn_blocking(tailscale_check_sync)
        .await
        .unwrap_or_else(|_| DoctorCheck {
            name: "tailscale".into(),
            ok: false,
            detail: "tailscale check panicked".into(),
            duration_ms: 0,
            payload: None,
            repair_hint: None,
        });
    check.duration_ms = start.elapsed().as_millis();
    check
}

fn tailscale_check_sync() -> DoctorCheck {
    let name = "tailscale".to_string();

    // Check if tailscale binary is present.
    if which::which("tailscale").is_err() {
        return DoctorCheck {
            name,
            ok: false,
            detail: "tailscale not found in PATH".into(),
            duration_ms: 0,
            payload: None,
            repair_hint: Some("Install at https://tailscale.com/download".into()),
        };
    }

    // Run `tailscale status --json` to get connection state.
    let output = match std::process::Command::new("tailscale")
        .args(["status", "--json"])
        .output()
    {
        Ok(o) => o,
        Err(err) => {
            return DoctorCheck {
                name,
                ok: false,
                detail: format!("tailscale status failed: {err}"),
                duration_ms: 0,
                payload: None,
                repair_hint: Some("Ensure tailscaled is running: sudo systemctl start tailscaled".into()),
            };
        }
    };

    let status_json: serde_json::Value = match serde_json::from_slice(&output.stdout) {
        Ok(v) => v,
        Err(_) => {
            let detail = if output.stdout.is_empty() {
                "tailscale daemon not responding (no output from status --json)".into()
            } else {
                "tailscale status --json returned unparseable output".into()
            };
            return DoctorCheck {
                name,
                ok: false,
                detail,
                duration_ms: 0,
                payload: None,
                repair_hint: Some("Check tailscaled health: sudo systemctl status tailscaled".into()),
            };
        }
    };

    let backend_state = status_json
        .get("BackendState")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown");

    match backend_state {
        "Running" => {
            let dns_name = status_json
                .get("Self")
                .and_then(|s| s.get("DNSName"))
                .and_then(|n| n.as_str())
                .map(|s| s.trim_end_matches('.').to_string())
                .unwrap_or_default();
            let ts_ip = status_json
                .get("Self")
                .and_then(|s| s.get("TailscaleIPs"))
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let detail = if dns_name.is_empty() {
                format!("Connected | {ts_ip}")
            } else {
                format!("Connected | {dns_name} | {ts_ip}")
            };
            DoctorCheck {
                name,
                ok: true,
                detail,
                duration_ms: 0,
                payload: Some(status_json),
                repair_hint: None,
            }
        }
        "NeedsLogin" => DoctorCheck {
            name,
            ok: false,
            detail: "Tailscale needs login".into(),
            duration_ms: 0,
            payload: Some(status_json),
            repair_hint: Some("tailscale up --authkey <key>".into()),
        },
        "Stopped" => DoctorCheck {
            name,
            ok: false,
            detail: "Tailscale daemon is stopped".into(),
            duration_ms: 0,
            payload: Some(status_json),
            repair_hint: Some("sudo systemctl start tailscaled".into()),
        },
        other => DoctorCheck {
            name,
            ok: false,
            detail: format!("Unexpected BackendState: {other}"),
            duration_ms: 0,
            payload: Some(status_json),
            repair_hint: Some("Check tailscale status for details".into()),
        },
    }
}

fn perform_repairs(config: &McConfig) -> Vec<DoctorRepair> {
    // re-use helpers from config module
    let mut repairs = Vec::new();
    match crate::config::ensure_mc_dirs() {
        Ok(()) => repairs.push(DoctorRepair::ok(
            "directories",
            format!(
                "Ensured MC_HOME={} and skills dir {}",
                crate::config::mc_home_dir().display(),
                crate::config::skills_home_dir().display()
            ),
        )),
        Err(err) => repairs.push(DoctorRepair::failed("directories", err.to_string())),
    }
    if config.agent_context.agent_id.is_none() {
        let agent_id = crate::config::default_agent_id_from_session(config.base_url.as_str())
            .unwrap_or_else(|| format!("mc-agent-{}", Uuid::new_v4()));
        match crate::config::persist_agent_id(&agent_id) {
            Ok(()) => repairs.push(DoctorRepair::ok(
                "agent_id",
                format!(
                    "Persisted agent_id {} at {}/agent_id",
                    agent_id,
                    crate::config::mc_home_dir().display()
                ),
            )),
            Err(err) => repairs.push(DoctorRepair::failed("agent_id", err.to_string())),
        }
    } else {
        repairs.push(DoctorRepair::ok(
            "agent_id",
            "Agent ID already configured".into(),
        ));
    }
    repairs
}

#[derive(serde::Serialize)]
struct DoctorReport {
    base_url: String,
    agent_id: Option<String>,
    matrix_endpoint: String,
    checks: Vec<DoctorCheck>,
    repairs: Vec<DoctorRepair>,
    cleanup: Option<ProfileGcSummary>,
}

#[derive(serde::Serialize)]
struct DoctorCheck {
    name: String,
    ok: bool,
    detail: String,
    duration_ms: u128,
    payload: Option<Value>,
    repair_hint: Option<String>,
}

#[derive(serde::Serialize)]
struct DoctorRepair {
    name: String,
    success: bool,
    detail: String,
}

impl DoctorRepair {
    fn ok(name: &str, detail: String) -> Self {
        Self {
            name: name.to_string(),
            success: true,
            detail,
        }
    }

    fn failed(name: &str, detail: String) -> Self {
        Self {
            name: name.to_string(),
            success: false,
            detail,
        }
    }
}

fn print_json(value: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
    );
}

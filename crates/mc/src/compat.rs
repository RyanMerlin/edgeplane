use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Args, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Subcommand, Debug)]
pub enum CompatCommand {
    /// Run compatibility checks for configured providers and emit a report artifact.
    #[command(subcommand)]
    Matrix(CompatMatrixCommand),
}

#[derive(Subcommand, Debug)]
pub enum CompatMatrixCommand {
    /// Execute compatibility checks and write a report.
    Run(CompatRunArgs),
    /// Print the latest compatibility report.
    ReportLatest,
}

#[derive(Args, Debug)]
pub struct CompatRunArgs {
    /// Providers to test. Comma-delimited values, e.g. claude,codex.
    #[arg(long, default_value = "claude,codex")]
    pub providers: String,

    /// Test depth profile.
    #[arg(long, value_enum, default_value_t = CompatMode::Smoke)]
    pub mode: CompatMode,

    /// Optional explicit output path for the report JSON.
    #[arg(long)]
    pub out: Option<PathBuf>,
}

#[derive(ValueEnum, Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompatMode {
    Smoke,
    Full,
}

#[derive(Debug, Serialize, Deserialize)]
struct CompatReport {
    schema: &'static str,
    run_id: String,
    generated_at: String,
    mode: CompatMode,
    overall_status: String,
    providers: Vec<ProviderResult>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ProviderResult {
    provider: String,
    available: bool,
    status: String,
    checks: Vec<CheckResult>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CheckResult {
    name: String,
    status: String,
    detail: String,
}

pub async fn run(command: CompatCommand) -> Result<()> {
    match command {
        CompatCommand::Matrix(inner) => match inner {
            CompatMatrixCommand::Run(args) => run_matrix(args),
            CompatMatrixCommand::ReportLatest => print_latest_report(),
        },
    }
}

fn run_matrix(args: CompatRunArgs) -> Result<()> {
    let providers = parse_providers(&args.providers);
    let mut results = Vec::with_capacity(providers.len());

    for provider in providers {
        results.push(run_provider_checks(provider, args.mode));
    }

    let overall_status = if results.iter().any(|r| r.status == "breaking") {
        "breaking"
    } else if results.iter().any(|r| r.status == "degraded") {
        "degraded"
    } else {
        "compatible"
    }
    .to_string();

    let report = CompatReport {
        schema: "mc/compat-report/v1",
        run_id: format!("compat-{}", Utc::now().timestamp_millis()),
        generated_at: Utc::now().to_rfc3339(),
        mode: args.mode,
        overall_status,
        providers: results,
    };

    let report_value = serde_json::to_value(&report)?;
    let path = if let Some(out) = args.out {
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&out, serde_json::to_vec_pretty(&report)?)?;
        out
    } else {
        persist_report(&report)?
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "ok": true,
            "artifact": "compat-report.json",
            "path": path,
            "summary": report_value,
        }))?
    );
    Ok(())
}

fn print_latest_report() -> Result<()> {
    let latest_path = compat_home().join("latest.json");
    let bytes = fs::read(&latest_path).with_context(|| {
        format!(
            "no latest compatibility report at {}",
            latest_path.display()
        )
    })?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

fn persist_report(report: &CompatReport) -> Result<PathBuf> {
    let report_dir = compat_home().join("reports");
    fs::create_dir_all(&report_dir)?;

    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    let file_name = format!("{}-{}.json", stamp, mode_label(report.mode));
    let report_path = report_dir.join(file_name);
    let data = serde_json::to_vec_pretty(report)?;
    fs::write(&report_path, &data)?;

    let latest_path = compat_home().join("latest.json");
    fs::create_dir_all(compat_home())?;
    fs::write(latest_path, &data)?;
    Ok(report_path)
}

fn mode_label(mode: CompatMode) -> &'static str {
    match mode {
        CompatMode::Smoke => "smoke",
        CompatMode::Full => "full",
    }
}

fn run_provider_checks(provider: String, mode: CompatMode) -> ProviderResult {
    let available = binary_available(&provider);
    let mut checks = Vec::new();

    checks.push(CheckResult {
        name: "binary_present".into(),
        status: if available { "pass" } else { "fail" }.into(),
        detail: if available {
            format!("{provider} executable resolved in PATH")
        } else {
            format!("{provider} executable missing from PATH")
        },
    });

    checks.push(CheckResult {
        name: "contract_payload_shape".into(),
        status: if available { "pass" } else { "warn" }.into(),
        detail: "placeholder contract check recorded for CI matrix aggregation".into(),
    });

    if mode == CompatMode::Full {
        checks.push(CheckResult {
            name: "approval_flow_contract".into(),
            status: if available { "pass" } else { "warn" }.into(),
            detail: "full-mode check placeholder for governance/approval contract".into(),
        });
    }

    let status = if !available { "degraded" } else { "compatible" }.to_string();

    ProviderResult {
        provider,
        available,
        status,
        checks,
    }
}

fn parse_providers(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    for token in input.split(',') {
        let trimmed = token.trim();
        if !trimmed.is_empty() {
            out.push(trimmed.to_string());
        }
    }
    if out.is_empty() {
        vec!["claude".to_string(), "codex".to_string()]
    } else {
        out
    }
}

fn binary_available(binary: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };

    std::env::split_paths(&paths).any(|dir| is_executable_path(&dir.join(binary)))
}

fn is_executable_path(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let Ok(meta) = fs::metadata(path) else {
            return false;
        };
        return meta.permissions().mode() & 0o111 != 0;
    }

    #[cfg(not(unix))]
    {
        true
    }
}

fn compat_home() -> PathBuf {
    crate::config::mc_home_dir().join("compat")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_providers_falls_back_to_defaults() {
        let parsed = parse_providers(" , ");
        assert_eq!(parsed, vec!["claude".to_string(), "codex".to_string()]);
    }

    #[test]
    fn provider_without_binary_is_degraded() {
        let result = run_provider_checks(
            "definitely-not-installed-binary".to_string(),
            CompatMode::Smoke,
        );
        assert!(!result.available);
        assert_eq!(result.status, "degraded");
        assert_eq!(result.checks[0].name, "binary_present");
    }
}

use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Args, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;

#[derive(Subcommand, Debug)]
pub enum DriftCommand {
    /// Ingest a provider change signal and persist a capability delta artifact.
    Ingest(DriftIngestArgs),
    /// Produce a policy decision from latest compatibility and drift artifacts.
    Triage(DriftTriageArgs),
}

#[derive(Args, Debug)]
pub struct DriftIngestArgs {
    /// Provider identifier (e.g. claude, codex).
    #[arg(long)]
    pub provider: String,

    /// Version label seen in docs/release notes.
    #[arg(long)]
    pub version: String,

    /// Source URL where change was observed.
    #[arg(long)]
    pub source_url: String,

    /// Human summary of the observed drift/change.
    #[arg(long)]
    pub summary: String,

    /// Drift severity classification.
    #[arg(long, value_enum, default_value_t = DriftSeverity::Degraded)]
    pub severity: DriftSeverity,
}

#[derive(Args, Debug)]
pub struct DriftTriageArgs {
    /// Optional mission id for bookkeeping.
    #[arg(long)]
    pub mission: Option<String>,

    /// Optional provider filter.
    #[arg(long)]
    pub provider: Option<String>,
}

#[derive(ValueEnum, Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DriftSeverity {
    Compatible,
    Degraded,
    Breaking,
}

#[derive(Debug, Serialize, Deserialize)]
struct CapabilityDelta {
    schema: &'static str,
    delta_id: String,
    provider: String,
    version: String,
    source_url: String,
    summary: String,
    severity: DriftSeverity,
    observed_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PolicyDecision {
    schema: &'static str,
    decision_id: String,
    mission: Option<String>,
    provider: Option<String>,
    compat_status: String,
    drift_severity: String,
    decision: String,
    rationale: String,
    generated_at: String,
}

pub async fn run(command: DriftCommand) -> Result<()> {
    match command {
        DriftCommand::Ingest(args) => ingest(args),
        DriftCommand::Triage(args) => triage(args),
    }
}

fn ingest(args: DriftIngestArgs) -> Result<()> {
    let provider = args.provider.trim().to_lowercase();
    if provider.is_empty() {
        anyhow::bail!("--provider cannot be empty");
    }

    let delta = CapabilityDelta {
        schema: "mc/capability-delta/v1",
        delta_id: format!("delta-{}", Utc::now().timestamp_millis()),
        provider: provider.clone(),
        version: args.version,
        source_url: args.source_url,
        summary: args.summary,
        severity: args.severity,
        observed_at: Utc::now().to_rfc3339(),
    };

    let path = persist_delta(&delta)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "ok": true,
            "artifact": "capability-delta.json",
            "path": path,
            "summary": serde_json::to_value(&delta)?,
        }))?
    );
    Ok(())
}

fn triage(args: DriftTriageArgs) -> Result<()> {
    let compat = load_latest_json(compat_latest_path()).unwrap_or_else(|_| {
        json!({
            "overall_status": "unknown"
        })
    });

    let drift = if let Some(provider) = &args.provider {
        load_latest_json(
            drift_home()
                .join("latest")
                .join(format!("{}.json", provider.trim().to_lowercase())),
        )
        .unwrap_or_else(|_| json!({"severity": "unknown", "provider": provider}))
    } else {
        load_latest_json(drift_home().join("latest.json"))
            .unwrap_or_else(|_| json!({"severity": "unknown"}))
    };

    let compat_status = compat
        .get("overall_status")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let drift_severity = drift
        .get("severity")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();

    let (decision, rationale) = decide(&compat_status, &drift_severity);
    let policy = PolicyDecision {
        schema: "mc/policy-decision/v1",
        decision_id: format!("decision-{}", Utc::now().timestamp_millis()),
        mission: args.mission,
        provider: args.provider,
        compat_status,
        drift_severity,
        decision: decision.to_string(),
        rationale,
        generated_at: Utc::now().to_rfc3339(),
    };

    let path = persist_decision(&policy)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "ok": true,
            "artifact": "policy-decision.json",
            "path": path,
            "summary": serde_json::to_value(&policy)?,
        }))?
    );
    Ok(())
}

fn decide(compat_status: &str, drift_severity: &str) -> (&'static str, String) {
    if compat_status == "breaking" || drift_severity == "breaking" {
        (
            "block",
            "breaking drift detected; require human approval before release".to_string(),
        )
    } else if compat_status == "degraded" || drift_severity == "degraded" {
        (
            "warn",
            "degraded behavior detected; allow staged rollout with warnings".to_string(),
        )
    } else {
        (
            "allow",
            "no breaking/degraded signals detected from compatibility loop".to_string(),
        )
    }
}

fn persist_delta(delta: &CapabilityDelta) -> Result<PathBuf> {
    let deltas_dir = drift_home().join("deltas");
    fs::create_dir_all(&deltas_dir)?;
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    let file = deltas_dir.join(format!("{}-{}.json", stamp, delta.provider));
    let bytes = serde_json::to_vec_pretty(delta)?;
    fs::write(&file, &bytes)?;

    let latest_dir = drift_home().join("latest");
    fs::create_dir_all(&latest_dir)?;
    fs::write(latest_dir.join(format!("{}.json", delta.provider)), &bytes)?;
    fs::write(drift_home().join("latest.json"), &bytes)?;
    Ok(file)
}

fn persist_decision(decision: &PolicyDecision) -> Result<PathBuf> {
    let decisions_dir = drift_home().join("decisions");
    fs::create_dir_all(&decisions_dir)?;
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    let file = decisions_dir.join(format!("{}.json", stamp));
    let bytes = serde_json::to_vec_pretty(decision)?;
    fs::write(&file, &bytes)?;
    fs::write(drift_home().join("latest-decision.json"), &bytes)?;
    Ok(file)
}

fn load_latest_json(path: PathBuf) -> Result<Value> {
    let bytes = fs::read(&path).with_context(|| format!("missing artifact {}", path.display()))?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn compat_latest_path() -> PathBuf {
    crate::config::mc_home_dir()
        .join("compat")
        .join("latest.json")
}

fn drift_home() -> PathBuf {
    crate::config::mc_home_dir().join("drift")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decide_blocks_on_breaking_drift() {
        let (decision, _) = decide("compatible", "breaking");
        assert_eq!(decision, "block");
    }

    #[test]
    fn decide_warns_on_degraded_compat() {
        let (decision, _) = decide("degraded", "compatible");
        assert_eq!(decision, "warn");
    }

    #[test]
    fn decide_allows_when_no_alerts() {
        let (decision, _) = decide("compatible", "compatible");
        assert_eq!(decision, "allow");
    }
}

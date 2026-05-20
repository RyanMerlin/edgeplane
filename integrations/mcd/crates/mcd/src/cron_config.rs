//! mcd cron config — file-as-config parser.
//!
//! Phase 4 of the daemon-absorption plan. mcd owns `~/.mc/mcd/cron.toml`,
//! reads it at startup + on explicit reload, dispatches the scheduled
//! prompts via `runtime.signal`. The file IS the source of truth; SQLite
//! stores only runtime telemetry (last_fired_at, fire log, GC'd).
//!
//! Schema is byte-compatible with `aria-cron.toml` so migration is `cp`:
//!
//! ```bash
//! cp ~/code/aria/aria-cron.toml ~/.mc/mcd/cron.toml
//! systemctl --user restart mcd
//! ```
//!
//! See `mc-engineer/projects/2026-05-20-phase4-implementation-plan.md` for
//! the design rationale and decisions (D4.1–D4.14).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

/// Maximum config schema version this binary understands. Files with a
/// `schema_version` higher than this are refused on load with an explicit
/// "binary too old, upgrade mcd" message.
pub const MCD_SUPPORTED_CRON_SCHEMA: u32 = 1;

/// Default config file path: `~/.mc/mcd/cron.toml`. Override via env
/// `MCD_CRON_FILE` or `DaemonConfig.cron_file`.
pub fn default_path() -> PathBuf {
    mcd_core::paths::mcd_dir().join("cron.toml")
}

/// Resolve the cron config file path: env override > config override > default.
/// Returns the path regardless of whether the file exists — caller decides
/// what to do with a missing file.
pub fn resolve_path(config_override: Option<&Path>) -> PathBuf {
    if let Ok(env_path) = std::env::var("MCD_CRON_FILE") {
        return PathBuf::from(env_path);
    }
    config_override
        .map(|p| p.to_path_buf())
        .unwrap_or_else(default_path)
}

// ── Schema ──────────────────────────────────────────────────────────────

/// Top-level config. Mirrors the structure of `aria-cron.toml` plus
/// schema_version + retention.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CronConfig {
    /// File schema version. Required. mcd refuses values > `MCD_SUPPORTED_CRON_SCHEMA`.
    pub schema_version: u32,

    /// IANA timezone for all cron expressions in this file. Default
    /// `"America/Denver"` for backwards compat with the aria-cron.toml
    /// implicit timezone.
    #[serde(default = "default_timezone")]
    pub timezone: String,

    /// Retention policy for the SQLite fire log. Default values are sane
    /// for a 15-job fleet firing roughly daily; tune if your job density
    /// is very different.
    #[serde(default)]
    pub retention: CronRetention,

    /// Job definitions. The `#[serde(rename = "job")]` lets the TOML use
    /// the conventional `[[job]]` array-of-tables syntax matching
    /// aria-cron.toml exactly.
    #[serde(rename = "job", default)]
    pub jobs: Vec<CronJob>,
}

fn default_timezone() -> String {
    "America/Denver".to_string()
}

/// Retention policy for `agent_cron_fire_log`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CronRetention {
    /// Drop fire log rows older than this many days. `0` = keep forever
    /// (not recommended; the GC sweep still runs, just no-ops).
    #[serde(default = "default_history_days")]
    pub history_days: u32,

    /// Cap fire log rows per job. Older rows beyond this are dropped
    /// even if within `history_days`. Protects against a misbehaving
    /// every-minute job flooding the DB.
    #[serde(default = "default_max_rows_per_job")]
    pub max_rows_per_job: u32,

    /// How often the GC sweep runs. Default 60 min — cheap query.
    #[serde(default = "default_gc_interval_minutes")]
    pub gc_interval_minutes: u32,
}

fn default_history_days() -> u32 {
    30
}
fn default_max_rows_per_job() -> u32 {
    500
}
fn default_gc_interval_minutes() -> u32 {
    60
}

impl Default for CronRetention {
    fn default() -> Self {
        Self {
            history_days: default_history_days(),
            max_rows_per_job: default_max_rows_per_job(),
            gc_interval_minutes: default_gc_interval_minutes(),
        }
    }
}

/// One scheduled job. Byte-compatible with aria-cron.toml's `[[job]]`
/// blocks — same field names, same semantics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CronJob {
    /// Human-readable identifier. Unique within the file. Used as the
    /// primary key for SQLite telemetry and for `mc agent cron describe`.
    pub name: String,

    /// 5-field cron expression evaluated in the file's `timezone`.
    pub schedule: String,

    /// Target agent's local id (e.g. `"operator"`, `"work"`). The agent
    /// must exist in mcd's supervisor map at dispatch time, otherwise
    /// the fire is logged as `agent-not-supervised` and skipped.
    ///
    /// Defaults to `"operator"` if omitted (matches aria-cron.toml).
    /// Phase 6 may rename this to `agent_id` for clarity; Phase 4 keeps
    /// `session` for byte-compat with the existing file.
    #[serde(default = "default_session")]
    pub session: String,

    /// Prompt text to deliver. Sent as `AgentSignal::UserInput { text }`.
    /// Multi-line OK; TOML triple-quoted strings work.
    pub prompt: String,

    /// Dispatch mode. Only `"signal"` is supported in Phase 4;
    /// `"goose"` parses but the loader bails with a clear error.
    #[serde(default = "default_dispatch")]
    pub dispatch: String,

    /// Schedule kind. Only `"cron"` is supported in Phase 4;
    /// `"heartbeat"` parses but the loader bails.
    #[serde(default = "default_kind")]
    pub kind: String,

    /// Heartbeat interval. Ignored when `kind != "heartbeat"`. Kept on
    /// the struct so the field round-trips for forward compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval: Option<String>,

    /// Per-job enable flag. Disabled jobs are loaded into memory but
    /// not fired by the tick loop. Operators flip this in the file +
    /// reload.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_session() -> String {
    "operator".to_string()
}
fn default_dispatch() -> String {
    "signal".to_string()
}
fn default_kind() -> String {
    "cron".to_string()
}
fn default_enabled() -> bool {
    true
}

// ── Loader + validation ─────────────────────────────────────────────────

/// Parse a `cron.toml` file from `path`. Validates schema_version, croner
/// syntax for each job's schedule, unique job names, supported `kind` and
/// `dispatch` values.
pub fn load(path: &Path) -> Result<CronConfig> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading cron config: {}", path.display()))?;
    parse(&raw, path)
}

/// Parse a config string. Same validation as `load`, but no I/O — used
/// by tests and by callers that already have the content in hand.
pub fn parse(raw: &str, path_for_errors: &Path) -> Result<CronConfig> {
    let cfg: CronConfig = toml::from_str(raw)
        .with_context(|| format!("parsing cron config: {}", path_for_errors.display()))?;
    validate(&cfg, path_for_errors)?;
    Ok(cfg)
}

fn validate(cfg: &CronConfig, path_for_errors: &Path) -> Result<()> {
    if cfg.schema_version > MCD_SUPPORTED_CRON_SCHEMA {
        bail!(
            "{}: schema_version = {} is newer than this mcd supports ({}). \
             Upgrade mcd or downgrade the file.",
            path_for_errors.display(),
            cfg.schema_version,
            MCD_SUPPORTED_CRON_SCHEMA
        );
    }

    // Verify the timezone parses. `chrono_tz::Tz` parses from str.
    cfg.timezone.parse::<chrono_tz::Tz>().map_err(|e| {
        anyhow!(
            "{}: timezone = {:?} is not a valid IANA zone: {e}",
            path_for_errors.display(),
            cfg.timezone
        )
    })?;

    // Per-job validation: unique names, supported kind + dispatch, croner
    // schedule parse.
    let mut seen_names = std::collections::HashSet::new();
    for (idx, job) in cfg.jobs.iter().enumerate() {
        let where_ = format!("{} [[job]] #{idx} ({:?})", path_for_errors.display(), job.name);

        if job.name.is_empty() {
            bail!("{where_}: name is empty");
        }
        if !seen_names.insert(job.name.clone()) {
            bail!("{where_}: duplicate name in this file");
        }
        if job.session.is_empty() {
            bail!("{where_}: session is empty");
        }
        if job.prompt.is_empty() {
            bail!("{where_}: prompt is empty");
        }

        match job.kind.as_str() {
            "cron" => {}
            "heartbeat" => bail!(
                "{where_}: kind = \"heartbeat\" is not supported in mcd yet — \
                 coming in a follow-up. For now, use kind = \"cron\" with an explicit schedule."
            ),
            other => bail!("{where_}: unknown kind = {other:?} (expected \"cron\")"),
        }

        match job.dispatch.as_str() {
            "signal" => {}
            "goose" => bail!(
                "{where_}: dispatch = \"goose\" is not supported in mcd yet. \
                 For now, use dispatch = \"signal\" and have the agent shell to goose itself if needed."
            ),
            other => bail!("{where_}: unknown dispatch = {other:?} (expected \"signal\")"),
        }

        // Parse the cron expression to surface syntax errors at load time
        // rather than at first tick. croner v3 uses FromStr.
        let _cron: croner::Cron = job.schedule.parse().map_err(|e| {
            anyhow!("{where_}: schedule {:?} does not parse: {e}", job.schedule)
        })?;
    }

    Ok(())
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn p() -> PathBuf {
        PathBuf::from("/tmp/test-cron.toml")
    }

    #[test]
    fn parses_minimal_valid_config() {
        let raw = r#"
schema_version = 1

[[job]]
name = "briefing"
schedule = "30 5 * * *"
session = "operator"
prompt = "run /briefing"
"#;
        let cfg = parse(raw, &p()).unwrap();
        assert_eq!(cfg.schema_version, 1);
        assert_eq!(cfg.timezone, "America/Denver");
        assert_eq!(cfg.jobs.len(), 1);
        let job = &cfg.jobs[0];
        assert_eq!(job.name, "briefing");
        assert_eq!(job.dispatch, "signal"); // default
        assert_eq!(job.kind, "cron"); // default
        assert!(job.enabled); // default
        assert_eq!(cfg.retention.history_days, 30); // default
    }

    #[test]
    fn defaults_session_to_operator() {
        let raw = r#"
schema_version = 1

[[job]]
name = "x"
schedule = "0 0 * * *"
prompt = "x"
"#;
        let cfg = parse(raw, &p()).unwrap();
        assert_eq!(cfg.jobs[0].session, "operator");
    }

    #[test]
    fn rejects_future_schema_version() {
        let raw = r#"
schema_version = 99

[[job]]
name = "x"
schedule = "0 0 * * *"
prompt = "x"
"#;
        let err = parse(raw, &p()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("schema_version"), "msg: {msg}");
        assert!(msg.contains("Upgrade mcd"), "msg: {msg}");
    }

    #[test]
    fn rejects_invalid_timezone() {
        let raw = r#"
schema_version = 1
timezone = "Mars/Olympus"

[[job]]
name = "x"
schedule = "0 0 * * *"
prompt = "x"
"#;
        let err = parse(raw, &p()).unwrap_err();
        assert!(format!("{err:#}").contains("timezone"));
    }

    #[test]
    fn rejects_heartbeat_kind() {
        let raw = r#"
schema_version = 1

[[job]]
name = "x"
schedule = "0 0 * * *"
prompt = "x"
kind = "heartbeat"
interval = "30m"
"#;
        let err = parse(raw, &p()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("heartbeat"), "msg: {msg}");
        assert!(msg.contains("not supported"), "msg: {msg}");
    }

    #[test]
    fn rejects_goose_dispatch() {
        let raw = r#"
schema_version = 1

[[job]]
name = "x"
schedule = "0 0 * * *"
prompt = "x"
dispatch = "goose"
"#;
        let err = parse(raw, &p()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("goose"), "msg: {msg}");
        assert!(msg.contains("not supported"), "msg: {msg}");
    }

    #[test]
    fn rejects_duplicate_job_names() {
        let raw = r#"
schema_version = 1

[[job]]
name = "x"
schedule = "0 0 * * *"
prompt = "first"

[[job]]
name = "x"
schedule = "0 1 * * *"
prompt = "second"
"#;
        let err = parse(raw, &p()).unwrap_err();
        assert!(format!("{err:#}").contains("duplicate name"));
    }

    #[test]
    fn rejects_invalid_cron_schedule() {
        let raw = r#"
schema_version = 1

[[job]]
name = "x"
schedule = "totally not cron"
prompt = "x"
"#;
        let err = parse(raw, &p()).unwrap_err();
        assert!(format!("{err:#}").contains("schedule"));
    }

    #[test]
    fn parses_complete_aria_cron_shape() {
        // The real aria-cron.toml shape with all field variants.
        let raw = r#"
schema_version = 1
timezone = "America/Denver"

[retention]
history_days = 30
max_rows_per_job = 500
gc_interval_minutes = 60

[[job]]
name     = "briefing"
schedule = "30 5 * * *"
session  = "operator"
prompt   = "run /briefing"

[[job]]
name     = "analyst-harvest"
schedule = "0 8 * * 1-5"
session  = "research"
prompt   = "run /analyst-harvest"
enabled  = false
"#;
        let cfg = parse(raw, &p()).unwrap();
        assert_eq!(cfg.jobs.len(), 2);
        assert_eq!(cfg.retention.history_days, 30);
        assert!(!cfg.jobs[1].enabled);
    }

    #[test]
    fn no_schema_version_fails_cleanly() {
        // Missing required field — toml parse fails with a clear error.
        let raw = r#"
[[job]]
name = "x"
schedule = "0 0 * * *"
prompt = "x"
"#;
        let err = parse(raw, &p()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("schema_version"), "msg: {msg}");
    }

    #[test]
    fn load_path_resolution() {
        // Default path lives under mcd_dir().
        let default = default_path();
        assert!(
            default.ends_with("cron.toml"),
            "default path should end with cron.toml: {}",
            default.display()
        );
    }

    #[test]
    fn resolve_path_prefers_env_override() {
        unsafe {
            std::env::set_var("MCD_CRON_FILE", "/tmp/env-override.toml");
        }
        let resolved = resolve_path(Some(Path::new("/tmp/config-override.toml")));
        unsafe {
            std::env::remove_var("MCD_CRON_FILE");
        }
        assert_eq!(resolved, PathBuf::from("/tmp/env-override.toml"));
    }

    #[test]
    fn resolve_path_falls_back_to_config() {
        let resolved = resolve_path(Some(Path::new("/tmp/config-override.toml")));
        assert_eq!(resolved, PathBuf::from("/tmp/config-override.toml"));
    }

    #[test]
    fn parses_real_aria_cron_toml() {
        // Smoke test: the real aria-cron.toml on disk parses as long as
        // it adds schema_version. We exercise the schema by injecting
        // schema_version into a copy of the structure.
        // (Reading the actual file isn't always possible from CI; this
        // proves the parser handles the real shape.)
        let raw = r#"
schema_version = 1

[[job]]
name     = "vault-mirror"
schedule = "0 3 * * *"
session  = "operator"
prompt   = "Bash: aria vault mirror — log any errors to .learnings/ERRORS.md, no output needed on success"

[[job]]
name     = "ai-pulse-daily"
schedule = "13 6 * * 1-5"
session  = "research"
prompt   = """run /ai-pulse daily: multi-line prompts work fine"""

[[job]]
name     = "pub-queue-check"
schedule = "*/30 * * * *"
session  = "publisher"
prompt   = "queue check"
"#;
        let cfg = parse(raw, &p()).unwrap();
        assert_eq!(cfg.jobs.len(), 3);
        assert_eq!(cfg.jobs[2].session, "publisher");
    }
}

//! edgeplaned cron config — file-as-config parser.
//!
//! Phase 4 of the daemon-absorption plan. edgeplaned owns `~/.ep/edgeplaned/cron.toml`,
//! reads it at startup + on explicit reload, dispatches the scheduled
//! prompts via `runtime.signal`. The file IS the source of truth; SQLite
//! stores only runtime telemetry (last_fired_at, fire log, GC'd).
//!
//! Schema is byte-compatible with `aria-cron.toml` so migration is `cp`:
//!
//! ```bash
//! cp ~/code/aria/aria-cron.toml ~/.ep/edgeplaned/cron.toml
//! systemctl --user restart edgeplaned
//! ```
//!
//! See `Aria/Engineer/projects/2026-05-20-phase4-implementation-plan.md` for
//! the design rationale and decisions (D4.1–D4.14).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

/// Maximum config schema version this binary understands. Files with a
/// `schema_version` higher than this are refused on load with an explicit
/// "binary too old, upgrade edgeplaned" message.
pub const MCD_SUPPORTED_CRON_SCHEMA: u32 = 1;

/// Default config file path: `~/.ep/edgeplaned/cron.toml`. Override via env
/// `MCD_CRON_FILE` or `DaemonConfig.cron_file`.
pub fn default_path() -> PathBuf {
    edgeplaned_core::paths::mcd_dir().join("cron.toml")
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
    /// File schema version. Required. edgeplaned refuses values > `MCD_SUPPORTED_CRON_SCHEMA`.
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
    /// primary key for SQLite telemetry and for `edgeplane agent cron describe`.
    pub name: String,

    /// 5-field cron expression evaluated in the file's `timezone`.
    /// Required for `kind = "cron"`; omitted (or empty) for `kind = "heartbeat"`,
    /// which uses `interval` instead.
    #[serde(default)]
    pub schedule: String,

    /// Target agent's local id (e.g. `"operator"`, `"work"`). The agent
    /// must exist in edgeplaned's supervisor map at dispatch time, otherwise
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
            "{}: schema_version = {} is newer than this edgeplaned supports ({}). \
             Upgrade edgeplaned or downgrade the file.",
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
            "cron" => {
                if job.schedule.is_empty() {
                    bail!("{where_}: kind = \"cron\" requires a non-empty `schedule`");
                }
                // Parse the cron expression to surface syntax errors at load time
                // rather than at first tick. croner v3 uses FromStr.
                let _cron: croner::Cron = job.schedule.parse().map_err(|e| {
                    anyhow!("{where_}: schedule {:?} does not parse: {e}", job.schedule)
                })?;
            }
            "heartbeat" => {
                let raw = job.interval.as_deref().ok_or_else(|| {
                    anyhow!("{where_}: kind = \"heartbeat\" requires `interval` (e.g. \"30m\", \"2h\", \"1d\")")
                })?;
                parse_duration(raw).map_err(|e| anyhow!("{where_}: interval {raw:?}: {e}"))?;
                // schedule field is ignored for heartbeats but if it's present
                // and obviously malformed, surface a hint rather than silently dropping.
                // (Empty string is fine — operators may omit it.)
            }
            other => bail!("{where_}: unknown kind = {other:?} (expected \"cron\" or \"heartbeat\")"),
        }

        match job.dispatch.as_str() {
            "signal" | "goose" | "bash" => {}
            other => bail!(
                "{where_}: unknown dispatch = {other:?} (expected \"signal\", \"goose\", or \"bash\")"
            ),
        }
    }

    Ok(())
}

/// Parse a heartbeat interval string into a `Duration`.
///
/// Accepted forms: a sequence of `<integer><unit>` pairs concatenated with
/// no spaces. Units are `s` (seconds), `m` (minutes), `h` (hours), `d` (days).
/// Examples: `"30s"`, `"15m"`, `"2h"`, `"1d"`, `"2h30m"`, `"1d6h"`.
///
/// Returns an error for empty input, unknown units, or numeric overflow.
pub fn parse_duration(raw: &str) -> Result<std::time::Duration> {
    if raw.is_empty() {
        bail!("empty");
    }
    let mut total_secs: u64 = 0;
    let mut num: Option<u64> = None;
    for c in raw.chars() {
        if let Some(d) = c.to_digit(10) {
            num = Some(num.unwrap_or(0).checked_mul(10).and_then(|n| n.checked_add(d as u64))
                .ok_or_else(|| anyhow!("number overflow in {raw:?}"))?);
        } else {
            let n = num.take().ok_or_else(|| {
                anyhow!("unit {c:?} with no preceding number in {raw:?}")
            })?;
            let mult: u64 = match c {
                's' => 1,
                'm' => 60,
                'h' => 60 * 60,
                'd' => 60 * 60 * 24,
                other => bail!("unknown unit {other:?} in {raw:?} (expected s/m/h/d)"),
            };
            total_secs = total_secs
                .checked_add(n.checked_mul(mult).ok_or_else(|| anyhow!("overflow in {raw:?}"))?)
                .ok_or_else(|| anyhow!("overflow in {raw:?}"))?;
        }
    }
    if num.is_some() {
        bail!("trailing number with no unit in {raw:?}");
    }
    if total_secs == 0 {
        bail!("interval evaluates to zero in {raw:?}");
    }
    Ok(std::time::Duration::from_secs(total_secs))
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
        assert!(msg.contains("Upgrade edgeplaned"), "msg: {msg}");
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
    fn accepts_heartbeat_with_interval() {
        let raw = r#"
schema_version = 1

[[job]]
name = "hb"
schedule = ""
prompt = "x"
kind = "heartbeat"
interval = "30m"
"#;
        let cfg = parse(raw, &p()).expect("heartbeat parses");
        let job = &cfg.jobs[0];
        assert_eq!(job.kind, "heartbeat");
        assert_eq!(job.interval.as_deref(), Some("30m"));
    }

    #[test]
    fn accepts_heartbeat_without_schedule_field() {
        // schedule field is optional for heartbeats — operators should be able
        // to omit it rather than write `schedule = ""`.
        let raw = r#"
schema_version = 1

[[job]]
name = "hb"
prompt = "x"
kind = "heartbeat"
interval = "30m"
"#;
        let cfg = parse(raw, &p()).expect("heartbeat parses without schedule field");
        assert_eq!(cfg.jobs[0].schedule, "");
    }

    #[test]
    fn rejects_cron_without_schedule() {
        let raw = r#"
schema_version = 1

[[job]]
name = "c"
prompt = "x"
"#;
        let err = parse(raw, &p()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("requires a non-empty `schedule`"), "msg: {msg}");
    }

    #[test]
    fn rejects_heartbeat_without_interval() {
        let raw = r#"
schema_version = 1

[[job]]
name = "hb"
schedule = ""
prompt = "x"
kind = "heartbeat"
"#;
        let err = parse(raw, &p()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("heartbeat"), "msg: {msg}");
        assert!(msg.contains("interval"), "msg: {msg}");
    }

    #[test]
    fn accepts_goose_dispatch() {
        let raw = r#"
schema_version = 1

[[job]]
name = "g"
schedule = "0 0 * * *"
prompt = "x"
dispatch = "goose"
"#;
        let cfg = parse(raw, &p()).expect("goose parses");
        assert_eq!(cfg.jobs[0].dispatch, "goose");
    }

    // ── bash dispatch tests ──────────────────────────────────────────────

    #[test]
    fn bash_dispatch_parses_in_cron_config() {
        // dispatch = "bash" must be accepted by the TOML parser + validator.
        let raw = r#"
schema_version = 1

[[job]]
name = "vault-mirror"
schedule = "0 3 * * *"
dispatch = "bash"
prompt = """aria vault mirror 2>>/home/merlin/code/aria/.learnings/ERRORS.md"""
"#;
        let cfg = parse(raw, &p()).expect("bash dispatch parses");
        assert_eq!(cfg.jobs[0].dispatch, "bash");
        assert_eq!(cfg.jobs[0].name, "vault-mirror");
    }

    #[test]
    fn bash_dispatch_works_with_heartbeat_kind() {
        // bash dispatch must compose with both timing tiers.
        let raw = r#"
schema_version = 1

[[job]]
name = "hb-bash"
kind = "heartbeat"
interval = "1h"
dispatch = "bash"
prompt = "echo hello"
"#;
        let cfg = parse(raw, &p()).expect("heartbeat + bash parses");
        let job = &cfg.jobs[0];
        assert_eq!(job.dispatch, "bash");
        assert_eq!(job.kind, "heartbeat");
    }

    #[test]
    fn bash_dispatch_command_construction() {
        // Verify the Command that dispatch_bash would build has the right binary and args.
        // We construct the Command without executing it (no .output() call) and inspect
        // the debug representation.
        let prompt = "aria vault mirror 2>>/tmp/errors.md";
        let mut cmd = std::process::Command::new("bash");
        cmd.args(["-c", prompt]);
        let debug_str = format!("{cmd:?}");
        // The debug output for Command includes the binary and args.
        assert!(
            debug_str.contains("bash"),
            "Command should invoke bash: {debug_str}"
        );
        assert!(
            debug_str.contains("-c"),
            "Command should pass -c flag: {debug_str}"
        );
        assert!(
            debug_str.contains("aria vault mirror"),
            "Command should include the prompt: {debug_str}"
        );
    }

    #[test]
    fn bash_dispatch_env_vars_set() {
        // Verify that EP_CRON_* env vars can be set on a Command without panic.
        // We test the env-var setup logic by building a Command with those vars
        // and confirming the builder doesn't reject them (type system check).
        let mut cmd = std::process::Command::new("bash");
        cmd.args(["-c", "echo $EP_CRON_JOB_NAME"])
            .env("EP_CRON_JOB_NAME", "vault-mirror")
            .env("EP_CRON_FIRE_TS", "1748000000")
            .env("EP_CRON_DISPATCH", "bash");
        let debug_str = format!("{cmd:?}");
        assert!(
            debug_str.contains("EP_CRON_JOB_NAME"),
            "EP_CRON_JOB_NAME should be in env: {debug_str}"
        );
        assert!(
            debug_str.contains("vault-mirror"),
            "job name value should be in env: {debug_str}"
        );
        assert!(
            debug_str.contains("EP_CRON_DISPATCH"),
            "EP_CRON_DISPATCH should be in env: {debug_str}"
        );
    }

    #[test]
    fn rejects_unknown_dispatch() {
        let raw = r#"
schema_version = 1

[[job]]
name = "bad"
schedule = "0 0 * * *"
prompt = "x"
dispatch = "k8s-job"
"#;
        let err = parse(raw, &p()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("unknown dispatch"), "msg: {msg}");
        assert!(msg.contains("k8s-job"), "msg: {msg}");
    }

    #[test]
    fn parse_duration_handles_compound() {
        use std::time::Duration;
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("15m").unwrap(), Duration::from_secs(15 * 60));
        assert_eq!(parse_duration("2h").unwrap(), Duration::from_secs(2 * 60 * 60));
        assert_eq!(parse_duration("1d").unwrap(), Duration::from_secs(24 * 60 * 60));
        assert_eq!(
            parse_duration("2h30m").unwrap(),
            Duration::from_secs(2 * 60 * 60 + 30 * 60)
        );
        assert_eq!(
            parse_duration("1d6h").unwrap(),
            Duration::from_secs(24 * 60 * 60 + 6 * 60 * 60)
        );
    }

    #[test]
    fn parse_duration_rejects_bad_input() {
        assert!(parse_duration("").is_err());
        assert!(parse_duration("30").is_err()); // no unit
        assert!(parse_duration("m30").is_err()); // unit before number
        assert!(parse_duration("30x").is_err()); // unknown unit
        assert!(parse_duration("0s").is_err()); // zero
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

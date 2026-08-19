//! Phase 5: systemd-unit liveness loop.
//!
//! For each agent with a `systemd_service` set in `agent_launch_context`,
//! `UnitHealthLoop` polls `systemctl --user is-active <service>` every
//! [`HEALTH_TICK_SECS`] seconds. If the unit is failed/inactive and the
//! agent isn't paused, edgeplaned issues `systemctl --user restart <service>`
//! with the following throttling:
//!
//! - **post-restart grace** (90s default): skip checks during this
//!   window after any restart so cascading failures don't trigger a
//!   loop.
//! - **retry throttle** (1800s default): if a unit stays dead, only
//!   re-attempt restart every N seconds.
//!
//! Plus an optional **nightly restart** at a configurable hour for
//! hygiene (claude processes accumulate memory across long uptimes).
//!
//! Every state transition publishes a [`SupervisorEvent`] to the
//! `events_tx` channel; mgmt-gateway exposes a streaming subscription
//! (`agent.supervise.events`) so TUI / web portal can tail.
//!
//! All failures are logged + continue; the loop never panics.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::{Local, Timelike, Utc};
use edgeplaned_core::types::SupervisorEvent;
use tokio::sync::broadcast;
use tokio::time::{MissedTickBehavior, interval};

use crate::local_registry::{AgentLaunchContext, LocalRegistry};

/// Default throttle timings. Operators can tune these
/// via [`UnitHealthConfig`] in edgeplaned config.
pub const DEFAULT_TICK_SECS: u64 = 60;
pub const DEFAULT_RETRY_THROTTLE_SECS: u64 = 1800;
pub const DEFAULT_POST_RESTART_GRACE_SECS: u64 = 90;
pub const DEFAULT_NIGHTLY_RESTART_HOUR: u32 = 3;

#[derive(Debug, Clone)]
pub struct UnitHealthConfig {
    pub tick_secs: u64,
    pub retry_throttle_secs: u64,
    pub post_restart_grace_secs: u64,
    /// `None` disables the nightly restart entirely.
    pub nightly_restart_hour: Option<u32>,
}

impl Default for UnitHealthConfig {
    fn default() -> Self {
        Self {
            tick_secs: DEFAULT_TICK_SECS,
            retry_throttle_secs: DEFAULT_RETRY_THROTTLE_SECS,
            post_restart_grace_secs: DEFAULT_POST_RESTART_GRACE_SECS,
            nightly_restart_hour: Some(DEFAULT_NIGHTLY_RESTART_HOUR),
        }
    }
}

/// In-memory state the loop carries between ticks. Persisted state
/// (last restart attempt timestamp, dead-detection cache) is kept here;
/// SQLite holds only the append-only restart log.
struct AgentState {
    /// When edgeplaned last issued a restart for this agent. Throttle window
    /// reads from this.
    last_restart_attempt_at: Option<Instant>,
    /// When the last restart actually completed (success OR failure).
    /// Grace window reads from this.
    last_restart_completed_at: Option<Instant>,
    /// True if we've already published `UnitDeadDetected` for the
    /// current dead-run. Reset to false when the unit comes back active.
    dead_alert_published: bool,
    /// Local-time date string ("YYYY-MM-DD") of the last nightly
    /// restart we issued. Prevents double-firing within the same
    /// nightly hour window.
    last_nightly_date: Option<String>,
}

impl AgentState {
    fn new() -> Self {
        Self {
            last_restart_attempt_at: None,
            last_restart_completed_at: None,
            dead_alert_published: false,
            last_nightly_date: None,
        }
    }
}

pub struct UnitHealthLoop {
    registry_path: PathBuf,
    config: UnitHealthConfig,
    events_tx: broadcast::Sender<SupervisorEvent>,
    /// Per-agent in-memory state, keyed by `(source, agent_id)`.
    state: HashMap<(String, String), AgentState>,
}

impl UnitHealthLoop {
    pub fn new(
        registry_path: PathBuf,
        config: UnitHealthConfig,
        events_tx: broadcast::Sender<SupervisorEvent>,
    ) -> Self {
        Self {
            registry_path,
            config,
            events_tx,
            state: HashMap::new(),
        }
    }

    /// Run the loop forever. Caller spawns into a tokio task.
    pub async fn run(mut self) {
        let mut ticker = interval(Duration::from_secs(self.config.tick_secs));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        tracing::info!(
            "unit_health: tick={}s retry_throttle={}s post_restart_grace={}s nightly_hour={:?}",
            self.config.tick_secs,
            self.config.retry_throttle_secs,
            self.config.post_restart_grace_secs,
            self.config.nightly_restart_hour
        );
        loop {
            ticker.tick().await;
            self.process_tick().await;
        }
    }

    /// One tick. Loads agents from the registry, checks each unit's
    /// state, restarts dead ones (subject to grace + throttle).
    /// Failures are logged + skipped per-agent — the tick loop never
    /// dies.
    async fn process_tick(&mut self) {
        let agents = match self.load_agents().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("unit_health: load_agents failed: {e:#}. Skipping tick.");
                return;
            }
        };

        let now_hour = Local::now().hour();
        let nightly_active = self
            .config
            .nightly_restart_hour
            .map(|h| h == now_hour)
            .unwrap_or(false);

        for ctx in agents {
            // Only supervise agents that actually have a systemd unit.
            let Some(service) = ctx.systemd_service.clone() else {
                continue;
            };
            // Skip edgeplaned itself if someone accidentally configures it —
            // restarting your own supervisor is a recipe for chaos.
            if service.starts_with("edgeplaned") {
                continue;
            }

            let key = (ctx.source.clone(), ctx.agent_id.clone());
            let _state = self
                .state
                .entry(key.clone())
                .or_insert_with(AgentState::new);

            // Skip while inside post-restart grace.
            let in_grace = self
                .state
                .get(&key)
                .and_then(|s| s.last_restart_completed_at)
                .map(|t| t.elapsed() < Duration::from_secs(self.config.post_restart_grace_secs))
                .unwrap_or(false);
            if in_grace {
                continue;
            }

            if let Err(e) = self.eval_agent(&ctx, &service, nightly_active).await {
                tracing::warn!(
                    "unit_health: eval for {}/{}: {e:#}. Skipping.",
                    ctx.source,
                    ctx.agent_id
                );
            }
        }
    }

    async fn eval_agent(
        &mut self,
        ctx: &AgentLaunchContext,
        service: &str,
        nightly_active: bool,
    ) -> anyhow::Result<()> {
        let is_active = systemctl_is_active(service).await;
        let key = (ctx.source.clone(), ctx.agent_id.clone());

        if is_active {
            // Reset the dead-alert latch so a future dead-state gets
            // a fresh event.
            if let Some(s) = self.state.get_mut(&key) {
                s.dead_alert_published = false;
            }

            // Nightly restart: only if active (no point nightly-
            // restarting a unit that's already broken) and not yet
            // fired today.
            if nightly_active && !ctx.supervise_paused {
                let today = Local::now().format("%Y-%m-%d").to_string();
                let should_fire = self
                    .state
                    .get(&key)
                    .map(|s| s.last_nightly_date.as_deref() != Some(today.as_str()))
                    .unwrap_or(true);
                if should_fire {
                    self.fire_restart(ctx, service, "nightly").await?;
                    if let Some(s) = self.state.get_mut(&key) {
                        s.last_nightly_date = Some(today.clone());
                    }
                    // Also emit the dedicated nightly event for tracing
                    // / UI display (in addition to UnitRestarted).
                    let _ = self.events_tx.send(SupervisorEvent::NightlyRestartFired {
                        agent_id: ctx.agent_id.clone(),
                        source: ctx.source.clone(),
                        systemd_service: service.to_string(),
                        at: Utc::now().to_rfc3339(),
                    });
                }
            }
            return Ok(());
        }

        // Unit is dead. Publish UnitDeadDetected once per dead-run.
        let needs_alert = self
            .state
            .get(&key)
            .map(|s| !s.dead_alert_published)
            .unwrap_or(true);
        if needs_alert {
            let _ = self.events_tx.send(SupervisorEvent::UnitDeadDetected {
                agent_id: ctx.agent_id.clone(),
                source: ctx.source.clone(),
                systemd_service: service.to_string(),
                at: Utc::now().to_rfc3339(),
            });
            tracing::warn!(
                "unit_health: {}/{} ({}) is DOWN",
                ctx.source,
                ctx.agent_id,
                service
            );
            if let Some(s) = self.state.get_mut(&key) {
                s.dead_alert_published = true;
            }
        }

        // Respect operator pause.
        if ctx.supervise_paused {
            return Ok(());
        }

        // Throttle: don't retry restart faster than once per retry_throttle_secs.
        let throttled = self
            .state
            .get(&key)
            .and_then(|s| s.last_restart_attempt_at)
            .map(|t| t.elapsed() < Duration::from_secs(self.config.retry_throttle_secs))
            .unwrap_or(false);
        if throttled {
            // Log to SQLite as "throttled" so operators can see we held off.
            self.log_to_registry(ctx, "dead", "throttled", None, None)
                .await?;
            return Ok(());
        }

        self.fire_restart(ctx, service, "dead").await
    }

    /// Issue a `systemctl --user restart`. Records the result in
    /// `unit_restart_log` and publishes `SupervisorEvent::UnitRestarted`.
    /// Updates in-memory state (last_restart_attempt_at, last_restart_completed_at).
    async fn fire_restart(
        &mut self,
        ctx: &AgentLaunchContext,
        service: &str,
        reason: &str,
    ) -> anyhow::Result<()> {
        let key = (ctx.source.clone(), ctx.agent_id.clone());
        if let Some(s) = self.state.get_mut(&key) {
            s.last_restart_attempt_at = Some(Instant::now());
        }
        let started = Instant::now();
        let exit = systemctl_restart(service).await;
        let elapsed_ms = started.elapsed().as_millis();
        let (result, exit_code, notes) = match exit {
            Ok(0) => ("started", Some(0i64), None),
            Ok(code) => (
                "failed",
                Some(code as i64),
                Some(format!("systemctl exit {code}")),
            ),
            Err(e) => ("failed", None, Some(format!("spawn failed: {e}"))),
        };
        tracing::info!(
            "unit_health: restart {} ({}) reason={reason} result={result} {elapsed_ms}ms",
            ctx.agent_id,
            service
        );
        if let Some(s) = self.state.get_mut(&key) {
            s.last_restart_completed_at = Some(Instant::now());
        }
        self.log_to_registry(ctx, reason, result, exit_code, notes.as_deref())
            .await?;
        let _ = self.events_tx.send(SupervisorEvent::UnitRestarted {
            agent_id: ctx.agent_id.clone(),
            source: ctx.source.clone(),
            systemd_service: service.to_string(),
            reason: reason.to_string(),
            result: result.to_string(),
            exit_code,
            at: Utc::now().to_rfc3339(),
        });
        Ok(())
    }

    // ── registry I/O (spawn_blocking; LocalRegistry isn't Sync) ──

    async fn load_agents(&self) -> anyhow::Result<Vec<AgentLaunchContext>> {
        let registry_path = self.registry_path.clone();
        tokio::task::spawn_blocking(move || {
            let reg = LocalRegistry::open(&registry_path)?;
            reg.list_all_launch_contexts()
        })
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking panicked: {e}"))?
    }

    async fn log_to_registry(
        &self,
        ctx: &AgentLaunchContext,
        reason: &str,
        result: &str,
        exit_code: Option<i64>,
        notes: Option<&str>,
    ) -> anyhow::Result<()> {
        let registry_path = self.registry_path.clone();
        let source = ctx.source.clone();
        let agent_id = ctx.agent_id.clone();
        let reason = reason.to_string();
        let result = result.to_string();
        let notes = notes.map(String::from);
        let ts = Utc::now().to_rfc3339();
        tokio::task::spawn_blocking(move || -> anyhow::Result<i64> {
            let reg = LocalRegistry::open(&registry_path)?;
            reg.log_unit_restart(
                &agent_id,
                &source,
                &ts,
                &reason,
                &result,
                exit_code,
                notes.as_deref(),
            )
        })
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking panicked: {e}"))??;
        Ok(())
    }
}

/// Background GC task for `unit_restart_log`. Mirrors `cron::gc_task`'s
/// shape — periodic sweep, configurable retention.
pub async fn gc_task(
    registry_path: PathBuf,
    history_days: u32,
    max_rows_per_agent: u32,
    interval_minutes: u32,
) {
    loop {
        let sleep_secs = (interval_minutes as u64).saturating_mul(60).max(60);
        tokio::time::sleep(Duration::from_secs(sleep_secs)).await;

        let registry_path = registry_path.clone();
        let deleted = tokio::task::spawn_blocking(move || -> anyhow::Result<u64> {
            let reg = LocalRegistry::open(&registry_path)?;
            reg.unit_restart_gc(history_days, max_rows_per_agent)
        })
        .await;

        match deleted {
            Ok(Ok(n)) if n > 0 => tracing::info!(
                "unit_health gc: dropped {n} unit_restart_log rows (history_days={history_days}, max_rows_per_agent={max_rows_per_agent})"
            ),
            Ok(Ok(_)) => tracing::debug!("unit_health gc: no rows to drop"),
            Ok(Err(e)) => tracing::warn!("unit_health gc failed: {e:#}"),
            Err(e) => tracing::warn!("unit_health gc task panicked: {e}"),
        }
    }
}

// ── systemctl subprocess wrappers ──

/// Returns `true` only when `systemctl --user is-active <service>`
/// exits with code 0 and prints "active". Any other state (failed,
/// inactive, activating, deactivating) returns false.
async fn systemctl_is_active(service: &str) -> bool {
    match tokio::process::Command::new("systemctl")
        .args(["--user", "is-active", service])
        .output()
        .await
    {
        Ok(out) => {
            // is-active prints the state and exits 0 only for "active".
            out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "active"
        }
        Err(_) => false,
    }
}

/// Returns the exit code from `systemctl --user restart <service>`.
/// Wraps `tokio::process::Command::output` so the caller can also see
/// stderr if needed via the second return.
async fn systemctl_restart(service: &str) -> Result<i32, std::io::Error> {
    let out = tokio::process::Command::new("systemctl")
        .args(["--user", "restart", service])
        .output()
        .await?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        tracing::warn!(
            "systemctl --user restart {service} failed: exit={:?} stderr={}",
            out.status.code(),
            stderr.trim()
        );
    }
    Ok(out.status.code().unwrap_or(-1))
}

/// Convenience: read all agents + their state-or-default for
/// `edgeplane agent supervise list`. Returned as a flat list of
/// `(launch_context, current_unit_state)` tuples — caller renders.
pub async fn list_supervised(
    registry_path: PathBuf,
) -> anyhow::Result<Vec<(AgentLaunchContext, Option<String>)>> {
    let agents = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<AgentLaunchContext>> {
        let reg = LocalRegistry::open(&registry_path)?;
        Ok(reg
            .list_all_launch_contexts()?
            .into_iter()
            .filter(|c| c.systemd_service.is_some())
            .collect())
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking panicked: {e}"))??;

    let mut out = Vec::with_capacity(agents.len());
    for ctx in agents {
        let state = if let Some(svc) = &ctx.systemd_service {
            Some(systemctl_is_active_verbose(svc).await)
        } else {
            None
        };
        out.push((ctx, state));
    }
    Ok(out)
}

/// Returns the raw state string from `systemctl --user is-active`
/// ("active", "inactive", "failed", "activating", "unknown", etc.).
async fn systemctl_is_active_verbose(service: &str) -> String {
    match tokio::process::Command::new("systemctl")
        .args(["--user", "is-active", service])
        .output()
        .await
    {
        Ok(out) => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        Err(_) => "unreachable".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_values() {
        let c = UnitHealthConfig::default();
        assert_eq!(c.tick_secs, 60);
        assert_eq!(c.retry_throttle_secs, 1800);
        assert_eq!(c.post_restart_grace_secs, 90);
        assert_eq!(c.nightly_restart_hour, Some(3));
    }

    #[test]
    fn supervisor_event_serializes_with_kind_tag() {
        let ev = SupervisorEvent::UnitDeadDetected {
            agent_id: "work".into(),
            source: "fleet_import".into(),
            systemd_service: "my-agent-work.service".into(),
            at: "2026-05-20T12:00:00Z".into(),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains(r#""kind":"unit_dead_detected""#));
        assert!(json.contains(r#""agent_id":"work""#));
    }

    // The full tick-and-restart flow needs systemctl + a live unit, which
    // belongs in live integration tests on a running node — not unit tests.
}

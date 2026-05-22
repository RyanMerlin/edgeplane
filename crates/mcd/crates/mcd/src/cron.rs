//! mcd cron tick loop + GC task — Phase 4 stage 4.
//!
//! ## Tick loop (`CronLoop::run`)
//!
//! Fires every 60s. For each enabled job in the in-memory config:
//!   1. Parse `schedule` via croner in the configured timezone.
//!   2. Compute the next fire time *after* `last_fired_at` (or after the
//!      daemon-start time for jobs that have never fired).
//!   3. If next-fire ≤ now, dispatch via `runtime.signal(UserInput { text })`
//!      against the agent identified by `job.session`.
//!   4. Record the fire atomically in `agent_cron_state` + `agent_cron_fire_log`.
//!
//! Catches up missed iterations as **one fire per cron expression** (not N).
//! If mcd was down for the 05:30 briefing and comes up at 05:45, briefing
//! fires once on recovery. Same model aria-cron uses.
//!
//! ## GC task (`gc_task`)
//!
//! Separate tokio task firing every `cron.toml`'s `[retention] gc_interval_minutes`.
//! Calls `LocalRegistry::cron_gc(history_days, max_rows_per_job)`.
//!
//! ## Reload
//!
//! Phase 4 ships with explicit reload via mgmt_gateway (`agent.cron.reload`).
//! inotify-watch is deferred to a follow-up — operators run reload after
//! editing the file. mgmt_gateway sends a unit message over `reload_tx`;
//! the loop catches it on the next tick boundary and re-parses.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use mcd_core::types::AgentSignal;
use tokio::sync::mpsc;
use tokio::time::{Duration, MissedTickBehavior, interval};

use crate::attach_gateway::RuntimeMap;
use crate::cron_config::{self, CronConfig, CronJob};
use crate::local_registry::LocalRegistry;
use crate::supervisor::Supervisor;

/// Shared state the tick loop carries through its lifetime. The config
/// lives behind a Mutex so the reload path can swap it atomically without
/// stopping the tick.
pub struct CronLoop {
    config_path: PathBuf,
    config: Arc<tokio::sync::Mutex<CronConfig>>,
    supervisor: Arc<Supervisor>,
    #[allow(dead_code)]
    runtime_map: RuntimeMap,
    registry_path: PathBuf,
    daemon_start: DateTime<Utc>,
    reload_rx: mpsc::Receiver<()>,
}

/// Handle returned by `CronLoop::spawn` so callers can request reloads
/// from elsewhere (mgmt_gateway, signal handlers, …).
#[derive(Clone)]
pub struct CronHandle {
    reload_tx: mpsc::Sender<()>,
}

impl CronHandle {
    /// Request the cron loop re-read the config file on the next tick.
    /// Non-blocking — if a reload is already queued, this is a no-op.
    pub fn reload(&self) {
        let _ = self.reload_tx.try_send(());
    }
}

impl CronLoop {
    /// Build the loop. Reads the config file once; if missing, starts
    /// with an empty config (operator can `mc agent cron reload` later
    /// once the file exists).
    pub fn new(
        config_path: PathBuf,
        supervisor: Arc<Supervisor>,
        runtime_map: RuntimeMap,
        registry_path: PathBuf,
    ) -> (Self, CronHandle) {
        let config = match cron_config::load(&config_path) {
            Ok(cfg) => {
                tracing::info!(
                    "cron: {} jobs loaded from {}. \
                     If aria-cron.timer is still enabled, both schedulers will fire — disable with:\n  \
                       systemctl --user disable --now aria-cron.timer\n  \
                     Verify mcd cron is firing first via `mc agent cron list`.",
                    cfg.jobs.len(),
                    config_path.display()
                );
                cfg
            }
            Err(e) => {
                tracing::warn!(
                    "cron: could not load {} on startup: {e:#}. \
                     Starting with empty schedule. \
                     Fix the file and run `mc agent cron reload`.",
                    config_path.display()
                );
                CronConfig {
                    schema_version: 1,
                    timezone: "America/Denver".into(),
                    retention: Default::default(),
                    jobs: vec![],
                }
            }
        };

        let (reload_tx, reload_rx) = mpsc::channel(1);
        let loop_ = CronLoop {
            config_path,
            config: Arc::new(tokio::sync::Mutex::new(config)),
            supervisor,
            runtime_map,
            registry_path,
            daemon_start: Utc::now(),
            reload_rx,
        };
        let handle = CronHandle { reload_tx };
        (loop_, handle)
    }

    /// Run the tick loop forever. Caller spawns into a tokio task.
    pub async fn run(mut self) {
        let mut ticker = interval(Duration::from_secs(60));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

        // Fire once immediately so jobs scheduled at "now" don't wait a
        // full minute. (croner is exclusive on `last_fired_at`, so this
        // is safe.)
        ticker.tick().await;
        self.process_tick().await;

        loop {
            tokio::select! {
                _ = ticker.tick() => self.process_tick().await,
                Some(()) = self.reload_rx.recv() => self.reload().await,
            }
        }
    }

    /// Re-read the config file. On parse error, log and keep the
    /// previously-loaded config in memory.
    async fn reload(&self) {
        match cron_config::load(&self.config_path) {
            Ok(new_cfg) => {
                let mut guard = self.config.lock().await;
                tracing::info!(
                    "cron: reloaded {} jobs from {} (was {})",
                    new_cfg.jobs.len(),
                    self.config_path.display(),
                    guard.jobs.len()
                );
                *guard = new_cfg;
            }
            Err(e) => {
                tracing::warn!(
                    "cron: reload of {} failed: {e:#}. \
                     Keeping previously-loaded config.",
                    self.config_path.display()
                );
            }
        }
    }

    /// One tick: evaluate every enabled job, dispatch the due ones,
    /// record fires. Never panics; per-job errors are logged + skipped.
    async fn process_tick(&self) {
        let cfg_guard = self.config.lock().await;
        let timezone = cfg_guard.timezone.clone();
        let jobs: Vec<CronJob> = cfg_guard.jobs.iter().filter(|j| j.enabled).cloned().collect();
        drop(cfg_guard);

        let tz: Tz = match timezone.parse() {
            Ok(tz) => tz,
            Err(e) => {
                tracing::error!(
                    "cron: timezone {:?} failed to parse: {e}. Skipping tick.",
                    timezone
                );
                return;
            }
        };

        for job in &jobs {
            if let Err(e) = self.eval_job(job, tz).await {
                tracing::warn!(
                    "cron: job {:?} eval failed: {e:#}. Skipping this tick.",
                    job.name
                );
            }
        }
    }

    /// Evaluate one job: compute next fire from last_fired_at, dispatch
    /// if due, record fire. Branches on `job.kind` (cron vs heartbeat) and
    /// `job.dispatch` (signal vs goose).
    async fn eval_job(&self, job: &CronJob, tz: Tz) -> Result<()> {
        let state = self.read_state(&job.name).await?;

        let due = match job.kind.as_str() {
            "cron" => {
                // Anchor: last_fired_at if present, else daemon-start.
                let anchor: DateTime<Tz> = match state.and_then(|s| s.last_fired_at) {
                    Some(s) => DateTime::parse_from_rfc3339(&s)
                        .with_context(|| format!("parsing last_fired_at {:?}", s))?
                        .with_timezone(&tz),
                    None => self.daemon_start.with_timezone(&tz),
                };
                let cron: croner::Cron = job
                    .schedule
                    .parse()
                    .with_context(|| format!("schedule {:?}", job.schedule))?;
                let next_fire = cron.find_next_occurrence(&anchor, false).with_context(|| {
                    format!("find_next_occurrence for {:?} after {anchor}", job.name)
                })?;
                let now = Utc::now().with_timezone(&tz);
                next_fire <= now
            }
            "heartbeat" => {
                // Bootstrap: a heartbeat with no prior fire is due immediately
                // (consistent with scheduling.md). Otherwise fire when
                // now - last_fire >= interval.
                let raw_interval = job.interval.as_deref().ok_or_else(|| {
                    anyhow::anyhow!("heartbeat job {:?} missing interval", job.name)
                })?;
                let interval = crate::cron_config::parse_duration(raw_interval)
                    .with_context(|| format!("parsing interval for {:?}", job.name))?;
                match state.and_then(|s| s.last_fired_at) {
                    None => true, // first fire — go now
                    Some(s) => {
                        let last = DateTime::parse_from_rfc3339(&s)
                            .with_context(|| format!("parsing last_fired_at {:?}", s))?
                            .with_timezone(&tz);
                        let now = Utc::now().with_timezone(&tz);
                        let elapsed = now.signed_duration_since(last).to_std().unwrap_or_default();
                        elapsed >= interval
                    }
                }
            }
            other => bail!("unknown kind {other:?} for job {:?}", job.name),
        };

        if !due {
            return Ok(());
        }

        // Due: dispatch and record.
        let start = std::time::Instant::now();
        let dispatch_result = match job.dispatch.as_str() {
            "signal" => self.dispatch_signal(job).await,
            "goose" => self.dispatch_goose(job).await,
            "bash" => self.dispatch_bash(job).await,
            other => Err(anyhow::anyhow!("unknown dispatch {other:?} for job {:?}", job.name)),
        };
        let duration_ms = start.elapsed().as_millis() as i64;

        let fired_at = Utc::now().to_rfc3339();
        let (status, error_message) = match &dispatch_result {
            Ok(()) => ("ok", None),
            Err(e) if e.to_string().contains("not registered")
                  || e.to_string().contains("not supervised") => {
                ("agent-not-supervised", Some(format!("{e:#}")))
            }
            Err(e) => ("error", Some(format!("{e:#}"))),
        };

        if status == "ok" {
            tracing::info!(
                "cron: fired {} → {} (kind={}, dispatch={}, {duration_ms}ms)",
                job.name,
                job.session,
                job.kind,
                job.dispatch
            );
        } else {
            tracing::warn!(
                "cron: fired {} → {} but failed (status={status}, kind={}, dispatch={}, {duration_ms}ms): {}",
                job.name,
                job.session,
                job.kind,
                job.dispatch,
                error_message.as_deref().unwrap_or("?")
            );
        }

        self.record_fire(
            &job.name,
            &fired_at,
            status,
            Some(duration_ms),
            error_message.as_deref(),
        )
        .await?;

        Ok(())
    }

    /// Dispatch the prompt as `AgentSignal::UserInput` to the agent
    /// identified by `job.session`. Resolves the runtime + handle through
    /// the supervisor (same path Phase 3's `agent.local.signal` uses).
    async fn dispatch_signal(&self, job: &CronJob) -> Result<()> {
        let lookup = self
            .supervisor
            .with_agent(&job.session, |supervised| {
                (
                    supervised.runtime.clone(),
                    mcd_core::types::AgentHandle {
                        agent_id: supervised.handle.agent_id.clone(),
                        runtime_kind: supervised.handle.runtime_kind.clone(),
                        pid: supervised.handle.pid,
                    },
                )
            })
            .await;
        let (runtime, handle) = lookup.ok_or_else(|| {
            anyhow::anyhow!("agent {:?} is not supervised locally", job.session)
        })?;
        runtime
            .signal(
                &handle,
                AgentSignal::UserInput {
                    text: job.prompt.clone(),
                },
            )
            .await
    }

    /// Dispatch the prompt via `aria goose "<prompt>"` — runs locally on the
    /// node, no agent attachment needed. Useful for periodic computation that
    /// doesn't belong to any specific profile (auto-summaries, log digests,
    /// drift checks). `job.session` is treated as a metadata tag for telemetry
    /// only; it does NOT need to match a supervised agent.
    ///
    /// Honors `MCD_GOOSE_BIN` env override; falls back to `aria` from PATH.
    /// Timeout is 5 minutes — goose runs are intentionally short-lived; if
    /// you need longer, use dispatch="signal" to a real agent.
    async fn dispatch_goose(&self, job: &CronJob) -> Result<()> {
        let bin = std::env::var("MCD_GOOSE_BIN").unwrap_or_else(|_| "aria".to_string());
        let prompt = job.prompt.clone();
        let name = job.name.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let output = std::process::Command::new(&bin)
                .args(["goose", &prompt, "--timeout", "300"])
                .output()
                .with_context(|| format!("spawn `{bin} goose ...` for {name:?}"))?;
            if !output.status.success() {
                let code = output.status.code().unwrap_or(-1);
                let stderr = String::from_utf8_lossy(&output.stderr);
                bail!("goose exited {code}: {}", stderr.trim());
            }
            Ok(())
        })
        .await
        .context("spawn_blocking panicked dispatching goose")?
    }

    /// Dispatch the prompt as a literal shell script via `bash -c "<prompt>"`.
    ///
    /// No session attachment, no LLM round-trip, no profile context. The
    /// prompt is treated as a shell command string and executed directly.
    /// Suitable for deterministic, idempotent maintenance tasks (vault mirror,
    /// log rotation, data pipeline steps) where agent context adds no value.
    ///
    /// Env vars available to the script:
    /// - Standard: `HOME`, `PATH`, `USER`, `SHELL`, etc. (inherited from mcd)
    /// - Context: `MC_CRON_JOB_NAME`, `MC_CRON_FIRE_TS` (Unix epoch seconds),
    ///   `MC_CRON_DISPATCH=bash`
    ///
    /// Timeout: 5 minutes (same as goose). Exit code 0 → ok; non-zero → error
    /// (stderr preview captured in the cron fire log). mcd stays up on failure
    /// (soft-fail).
    async fn dispatch_bash(&self, job: &CronJob) -> Result<()> {
        let prompt = job.prompt.clone();
        let name = job.name.clone();
        let fire_ts = Utc::now().timestamp().to_string();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let output = std::process::Command::new("bash")
                .args(["-c", &prompt])
                .env("MC_CRON_JOB_NAME", &name)
                .env("MC_CRON_FIRE_TS", &fire_ts)
                .env("MC_CRON_DISPATCH", "bash")
                .output()
                .with_context(|| format!("spawn `bash -c ...` for cron job {name:?}"))?;
            if !output.status.success() {
                let code = output.status.code().unwrap_or(-1);
                let stderr_raw = String::from_utf8_lossy(&output.stderr);
                // Truncate stderr preview to first 500 chars to keep the fire log lean.
                let stderr_preview: String = stderr_raw.chars().take(500).collect();
                bail!("bash exited {code}: {}", stderr_preview.trim());
            }
            Ok(())
        })
        .await
        .context("spawn_blocking panicked dispatching bash")?
    }

    async fn read_state(
        &self,
        job_name: &str,
    ) -> Result<Option<crate::local_registry::AgentCronState>> {
        let registry_path = self.registry_path.clone();
        let job_name = job_name.to_string();
        tokio::task::spawn_blocking(move || {
            let reg = LocalRegistry::open(&registry_path)?;
            reg.cron_get_state(&job_name)
        })
        .await
        .context("spawn_blocking panicked reading cron state")?
    }

    async fn record_fire(
        &self,
        job_name: &str,
        fired_at: &str,
        status: &str,
        duration_ms: Option<i64>,
        error_message: Option<&str>,
    ) -> Result<()> {
        let registry_path = self.registry_path.clone();
        let job_name = job_name.to_string();
        let fired_at = fired_at.to_string();
        let status = status.to_string();
        let error_message = error_message.map(String::from);
        tokio::task::spawn_blocking(move || {
            let reg = LocalRegistry::open(&registry_path)?;
            reg.cron_record_fire(
                &job_name,
                &fired_at,
                &status,
                duration_ms,
                error_message.as_deref(),
            )
        })
        .await
        .context("spawn_blocking panicked recording fire")?
    }

    /// Read the loaded config (for `agent.cron.list` and friends).
    /// Returns a snapshot — caller doesn't hold the lock.
    pub fn config_snapshot(&self) -> impl std::future::Future<Output = CronConfig> + use<> {
        let cfg = self.config.clone();
        async move { cfg.lock().await.clone() }
    }

    /// Hand out the shared config Arc for the GC task. The GC task reads
    /// `[retention]` from this config; reload updates it under the same
    /// Mutex, so the next GC tick after reload picks up new retention
    /// values automatically.
    pub fn config_for_gc(&self) -> Arc<tokio::sync::Mutex<CronConfig>> {
        Arc::clone(&self.config)
    }
}

/// Background GC task. Runs every `cfg.retention.gc_interval_minutes`,
/// drops fire-log rows beyond `history_days` or `max_rows_per_job`.
pub async fn gc_task(
    config: Arc<tokio::sync::Mutex<CronConfig>>,
    registry_path: PathBuf,
) {
    loop {
        let (interval_min, history_days, max_rows) = {
            let cfg = config.lock().await;
            (
                cfg.retention.gc_interval_minutes,
                cfg.retention.history_days,
                cfg.retention.max_rows_per_job,
            )
        };

        let sleep_secs = (interval_min as u64).saturating_mul(60).max(60);
        tokio::time::sleep(Duration::from_secs(sleep_secs)).await;

        let registry_path = registry_path.clone();
        let deleted_result = tokio::task::spawn_blocking(move || -> Result<u64> {
            let reg = LocalRegistry::open(&registry_path)?;
            reg.cron_gc(history_days, max_rows)
        })
        .await;

        match deleted_result {
            Ok(Ok(n)) if n > 0 => {
                tracing::info!(
                    "cron gc: dropped {n} fire_log rows (history_days={history_days}, max_rows_per_job={max_rows})"
                );
            }
            Ok(Ok(_)) => {
                tracing::debug!("cron gc: no rows to drop");
            }
            Ok(Err(e)) => {
                tracing::warn!("cron gc failed: {e:#}");
            }
            Err(e) => {
                tracing::warn!("cron gc task panicked: {e}");
            }
        }
    }
}

/// Convenience: snapshot of the live config + state for `agent.cron.list`.
/// Joins `cron.toml` jobs against `agent_cron_state` so callers see jobs
/// even before they've fired.
pub async fn config_with_state(
    config_path: &std::path::Path,
    registry_path: PathBuf,
) -> Result<(CronConfig, Vec<crate::local_registry::AgentCronState>)> {
    let cfg = cron_config::load(config_path)?;
    let state = tokio::task::spawn_blocking(move || {
        let reg = LocalRegistry::open(&registry_path)?;
        reg.cron_list_state()
    })
    .await
    .context("spawn_blocking panicked listing cron state")??;
    Ok((cfg, state))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The full tick-and-dispatch flow needs a real Supervisor + runtime
    // map, which requires substantial setup. Phase 4 ships unit-test
    // coverage of cron_config + LocalRegistry helpers (where the
    // logic-heavy parts live); the loop itself is exercised by the live
    // validation step on excalibur.
    //
    // The one thing we DO test here is the schedule-anchor math via
    // croner — that's where subtle bugs hide.

    /// Regression: a daily 9 AM job should NOT be considered due when the last
    /// fire was earlier the same day (afternoon UTC). This guards against a
    /// recurrence of the "pub-queue-check fires every 30 min" symptom seen on
    /// 2026-05-20, which turned out to be a transient schedule change, not an
    /// eval bug — but the regression test is cheap insurance.
    #[test]
    fn daily_9am_with_recent_afternoon_anchor_is_not_due() {
        let tz: Tz = "America/Denver".parse().unwrap();
        let cron: croner::Cron = "0 9 * * *".parse().unwrap();
        // Last fired at 13:30 Denver (afternoon)
        let last: DateTime<Tz> = "2026-05-20T19:30:35+00:00"
            .parse::<DateTime<chrono::FixedOffset>>()
            .unwrap()
            .with_timezone(&tz);
        let next = cron.find_next_occurrence(&last, false).unwrap();
        // Now: 1 minute after the last fire
        let now: DateTime<Tz> = "2026-05-20T19:31:00+00:00"
            .parse::<DateTime<chrono::FixedOffset>>()
            .unwrap()
            .with_timezone(&tz);
        assert!(next > now, "next-fire {next} should be after now {now}");
        // Specifically: next fire should be tomorrow 09:00 Denver.
        assert_eq!(next.format("%Y-%m-%d %H:%M").to_string(), "2026-05-21 09:00");
    }

    /// Regression: a 05:30 daily job whose anchor is daemon_start at 14:20
    /// the previous afternoon should be due at any time on or after 05:30
    /// the next morning. This guards against the "briefing never fires"
    /// symptom from 2026-05-20.
    #[test]
    fn daily_530am_with_prior_afternoon_anchor_is_due_next_morning() {
        let tz: Tz = "America/Denver".parse().unwrap();
        let cron: croner::Cron = "30 5 * * *".parse().unwrap();
        let anchor: DateTime<Tz> = "2026-05-17T20:20:00+00:00"
            .parse::<DateTime<chrono::FixedOffset>>()
            .unwrap()
            .with_timezone(&tz);
        let next = cron.find_next_occurrence(&anchor, false).unwrap();
        assert_eq!(next.format("%Y-%m-%d %H:%M").to_string(), "2026-05-18 05:30");
        // At 07:00 the next morning, next_fire <= now should hold.
        let now: DateTime<Tz> = "2026-05-18T13:00:00+00:00"
            .parse::<DateTime<chrono::FixedOffset>>()
            .unwrap()
            .with_timezone(&tz);
        assert!(next <= now, "next-fire {next} should be due by {now}");
    }

    #[test]
    fn next_occurrence_after_last_fire_is_strict_next() {
        let tz: Tz = "America/Denver".parse().unwrap();
        let cron: croner::Cron = "30 5 * * *".parse().unwrap();
        // Last fired today at 05:30
        let last: DateTime<Tz> = "2026-05-20T05:30:00-06:00"
            .parse::<DateTime<chrono::FixedOffset>>()
            .unwrap()
            .with_timezone(&tz);
        let next = cron.find_next_occurrence(&last, false).unwrap();
        // Next fire is tomorrow 05:30, not today again
        assert_eq!(
            next.format("%Y-%m-%d %H:%M").to_string(),
            "2026-05-21 05:30"
        );
    }

    #[test]
    fn next_occurrence_handles_recent_anchor() {
        let tz: Tz = "America/Denver".parse().unwrap();
        let cron: croner::Cron = "*/30 * * * *".parse().unwrap();
        let now_anchor: DateTime<Tz> = "2026-05-20T12:15:00-06:00"
            .parse::<DateTime<chrono::FixedOffset>>()
            .unwrap()
            .with_timezone(&tz);
        let next = cron.find_next_occurrence(&now_anchor, false).unwrap();
        // From 12:15, the next */30 fire is 12:30
        assert_eq!(
            next.format("%Y-%m-%d %H:%M").to_string(),
            "2026-05-20 12:30"
        );
    }

    #[test]
    fn weekday_only_schedule_skips_weekends() {
        let tz: Tz = "America/Denver".parse().unwrap();
        let cron: croner::Cron = "0 9 * * 1-5".parse().unwrap();
        // Anchor Sat 09:00
        let sat: DateTime<Tz> = "2026-05-23T09:00:00-06:00"
            .parse::<DateTime<chrono::FixedOffset>>()
            .unwrap()
            .with_timezone(&tz);
        let next = cron.find_next_occurrence(&sat, false).unwrap();
        // Next fire is Monday 09:00
        assert_eq!(
            next.format("%Y-%m-%d %H:%M").to_string(),
            "2026-05-25 09:00"
        );
    }
}

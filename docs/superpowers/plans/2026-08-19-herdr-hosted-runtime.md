# Herdr-Hosted Runtime + PTY Bridge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `HerdrHostedRuntime` to edgeplaned — a thin facade (mirroring the existing `ZellijHostedRuntime`) that lets EdgePlane's own signal-delivery and PTY-attach surfaces reach an Aria profile hosted in a Herdr session, fixing the live bug where `edgeplane agent signal` and `edgeplane agent attach` silently fail (60s WARN-loop) for every Aria profile migrated off Zellij.

**Architecture:** EdgePlane does not host these agents — Aria's own systemd units + Herdr do (see the separate `aria` repo's `docs/superpowers/specs/2026-08-18-herdr-fleet-migration-v2.md`). This plan only teaches EdgePlane's existing `AgentRuntime` abstraction a second "poke an externally-hosted pane" implementation, parallel to `ZellijHostedRuntime`, using `herdr agent prompt --wait` (atomic submit + lifecycle-state wait) instead of Zellij's blind paste+sleep+Enter. EdgePlane remains a service Aria connects to, not the thing that owns Aria's session hosting — see the `RuntimeKind::ClaudeAgentAcp` doc comment ("the intended runtime for the Aria fleet") for a *different*, more centralized model that exists elsewhere in this codebase's history; this plan deliberately does not pursue that, it only extends the existing thin-facade pattern.

**Tech Stack:** Rust, tokio, rusqlite (SQLite registry), `herdr` CLI (pinned per-node, not vendored), `portable_pty` for the terminal bridge.

**Spec:** `/home/merlin/code/aria/docs/superpowers/specs/2026-08-14-herdr-fleet-migration.md` §8 ("Phase E") for the original ask. **This plan supersedes that section's file/method names** — investigation found the actual codebase differs from what that spec assumed in three important ways (see "Corrections to the original spec" below). Also: `Aria/Operator/plans/2026-08-19-herdr-evaluation-and-phase-e-recommendation.md` (vault) for the cross-checked findings that motivated this work.

## Corrections to the original spec

1. **`fleet_import.rs` (edgeplaned-bin) is dead code.** Every public item is `#[allow(dead_code)]`, and `import_into`/`load_profiles` have zero callers outside their own unit tests. It is NOT what `edgeplane daemon agent import` actually runs.
2. **The real, live import path is `crates/edgeplane/src/local_db.rs::import_manifest()`**, invoked from `daemon_ctl.rs`'s `Import(AgentImportArgs)` subcommand. It reads/writes `~/.ep/edgeplaned/registry.db` directly (standalone mode — no daemon round-trip).
3. **The registry schema is hand-mirrored across two independently-maintained files** with no shared migration code: `edgeplaned-bin/src/local_registry.rs` (versioned migrations, used by the daemon) and `edgeplane/src/local_db.rs` (a single `CREATE TABLE IF NOT EXISTS` literal, used by the CLI). Both must be updated for any schema change, or the two processes will disagree about what columns exist.
4. **`zellij_session` also lives on `LaunchContext`** (`edgeplaned-core/src/types.rs:174`) and flows through `SpawnOverrides` (`supervisor.rs`) from **two separate construction sites in `daemon.rs`** (the federated-merge path around line 1209, and the standalone-listing path around line 1672). A Herdr equivalent needs all of these, not just the registry column — a spec that only added the column would leave the runtime with no way to actually learn which Herdr session to talk to.

## Global Constraints

- Every task's commit must pass, in order: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo nextest run --workspace --locked`. (Learned the hard way on a sibling repo today: clippy+nextest passing locally is not sufficient — CI's fmt check is separate and will fail silently if skipped locally.)
- No Aria-specific paths (e.g. `~/.aria/bin`) anywhere in this repo — edgeplaned is a standalone product. Binary resolution mirrors `zellij_candidates()`'s generic probe order (bare name on PATH, `~/.cargo/bin`, `~/.local/bin`, `/usr/local/bin`, `/usr/bin`).
- Herdr session targeting is via the `HERDR_SESSION` environment variable on the spawned `Command`, **not** a `--session` CLI flag (Zellij's convention). Every subprocess call in `herdr_session.rs` sets it.
- `AgentSignal::Cancel` uses the herdr key name `ctrl+c` (lowercase, plus-joined) — do not copy Zellij's `"Ctrl c"` string, herdr's key-name grammar is different (confirmed live: `ctrl+u` worked as a real keypress against a running Herdr pane during this session's own verification work).
- Follow the existing `*_argv()` testability pattern (`zellij_session.rs`): every method that shells out builds its argv via a separate `Vec<String>`-returning helper, unit-tested without a subprocess; only integration-style tests (not run in default `cargo test`) touch a live `herdr`.
- The schema-duplication wart (constraint 3 above) is a known, pre-existing architectural issue. This plan works within it (updates both files) rather than fixing it — a follow-up to unify them is out of scope here.
- Task 10's live validation must never target an actively-working Aria pane, and must use a synthetic marker string, never `/clear` or anything state-destructive.

---

### Task 1: `RuntimeKind::HerdrHosted` + `LaunchContext.herdr_session`

**Files:**
- Modify: `crates/edgeplaned/crates/edgeplaned-core/src/types.rs`
- Test: same file, `#[cfg(test)]` module (create one if absent near the enum)

**Interfaces:**
- Produces: `RuntimeKind::HerdrHosted` (matches on the string `"herdr_hosted"` via its `Display` impl — every later task's string-matching code must use exactly that spelling), `LaunchContext.herdr_session: Option<String>`.

- [ ] **Step 1: Write the failing test**

Add near the existing `RuntimeKind` tests (or create a `#[cfg(test)] mod tests` right after the `Display` impl if none exists):

```rust
#[test]
fn herdr_hosted_display_is_snake_case() {
    assert_eq!(RuntimeKind::HerdrHosted.to_string(), "herdr_hosted");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p edgeplaned-core herdr_hosted_display_is_snake_case`
Expected: FAIL — `no variant named HerdrHosted found for enum RuntimeKind`

- [ ] **Step 3: Add the variant, Display arm, and LaunchContext field**

In the `RuntimeKind` enum (right after the `ZellijHosted` variant, before `Custom(String)`):

```rust
    /// A long-running agent hosted in a Herdr session. Mirrors ZellijHosted:
    /// the supervisor talks to the agent through `herdr` subprocess
    /// invocations (session-scoped via the `HERDR_SESSION` env var, not a
    /// `--session` flag), and there is no PTY owned by edgeplaned directly —
    /// see `edgeplaned-bin/src/herdr_bridge.rs` for the separate PTY-view
    /// path. The per-agent session name lives in
    /// `AgentLaunchContext.herdr_session`. See
    /// `edgeplaned-runtimes/src/herdr_hosted.rs`.
    HerdrHosted,
```

In the `Display` impl, add before the `Custom(s)` arm:

```rust
            RuntimeKind::HerdrHosted => write!(f, "herdr_hosted"),
```

In `LaunchContext`, right after the existing `zellij_session` field:

```rust
    /// Name of the Herdr session this agent runs in. Populated by the
    /// daemon from `AgentLaunchContext.herdr_session` only when
    /// `runtime_kind == HerdrHosted`. `None` for all other runtimes.
    pub herdr_session: Option<String>,
```

- [ ] **Step 4: Fix every other `LaunchContext { .. }` struct literal that doesn't use `..Default::default()`**

`cargo build -p edgeplaned-core -p edgeplaned-runtimes -p edgeplaned-bin 2>&1 | grep "missing field"` — add `herdr_session: None,` to each reported literal (expect hits in `supervisor.rs`'s test module and `zellij_hosted.rs`'s test module, both currently using `..Default::default()` for `LaunchContext` per the code already read, so this may be a no-op — verify either way).

- [ ] **Step 5: Run test to verify it passes, then the full local build**

Run: `cargo test -p edgeplaned-core herdr_hosted_display_is_snake_case && cargo build --workspace`
Expected: PASS, clean build.

- [ ] **Step 6: Commit**

```bash
cd ~/code/edgeplane
git add crates/edgeplaned/crates/edgeplaned-core/src/types.rs
git commit -m "feat(edgeplaned-core): add RuntimeKind::HerdrHosted + LaunchContext.herdr_session"
```

---

### Task 2: `herdr_session.rs` — subprocess wrapper

**Files:**
- Create: `crates/edgeplaned/crates/edgeplaned-runtimes/src/herdr_session.rs`
- Modify: `crates/edgeplaned/crates/edgeplaned-runtimes/src/lib.rs` (add `pub mod herdr_session;`)
- Modify: `crates/edgeplaned/crates/edgeplaned-runtimes/Cargo.toml` (add `serde_json` as a dependency if not already present — check first: `grep serde_json crates/edgeplaned/crates/edgeplaned-runtimes/Cargo.toml`)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `pub struct HerdrSession { pub name: String }` with `new`, `is_alive`, `discover_pane_id`, `send_prompt(pane_id, text)`, `send_keys(pane_id, keys)`, `dump_screen(pane_id, lines: Option<u32>)`, plus argv-builder helpers `prompt_argv`, `send_keys_argv`, `read_argv`; `pub const DEFAULT_PANE_ID: &str = "w1:p1"`; `pub fn herdr_candidates() -> Vec<String>`.

- [ ] **Step 1: Write the failing argv-shape tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_argv_shape() {
        let s = HerdrSession::new("vega");
        let argv = s.prompt_argv("w1:p1", "hello world");
        assert_eq!(
            argv,
            vec![
                "agent", "prompt", "w1:p1", "hello world",
                "--wait", "--until", "working", "--until", "blocked",
                "--timeout", "15000",
            ]
        );
    }

    #[test]
    fn send_keys_argv_single_key() {
        let s = HerdrSession::new("vega");
        let argv = s.send_keys_argv("w1:p1", &["ctrl+c"]);
        assert_eq!(argv, vec!["pane", "send-keys", "w1:p1", "ctrl+c"]);
    }

    #[test]
    fn read_argv_default_has_no_lines_flag() {
        let s = HerdrSession::new("vega");
        let argv = s.read_argv("w1:p1", None);
        assert_eq!(
            argv,
            vec!["pane", "read", "w1:p1", "--source", "recent-unwrapped"]
        );
    }

    #[test]
    fn read_argv_with_lines() {
        let s = HerdrSession::new("vega");
        let argv = s.read_argv("w1:p1", Some(200));
        assert_eq!(argv.last().unwrap(), "200");
        assert!(argv.contains(&"--lines".to_string()));
    }

    #[test]
    fn is_alive_parses_session_list_json() {
        let json = r#"{"sessions":[
            {"name":"vega","running":true},
            {"name":"gwyn","running":false}
        ]}"#;
        let parsed: SessionListOutput = serde_json::from_str(json).unwrap();
        assert!(parsed.sessions.iter().any(|s| s.name == "vega" && s.running));
        assert!(!parsed.sessions.iter().any(|s| s.name == "gwyn" && s.running));
    }

    #[test]
    fn agent_list_envelope_parses_real_shape() {
        // Captured live 2026-08-18 from `HERDR_SESSION=merlinlabs herdr agent list`.
        let json = r#"{"id":"cli:agent:list","result":{"agents":[{"agent":"claude",
            "agent_status":"idle","cwd":"/home/merlin/code/aria/profiles/merlinlabs",
            "focused":true,"foreground_cwd":"/tmp","pane_id":"w1:p1","revision":1,
            "state_change_seq":1,"tab_id":"w1:t1","terminal_id":"term_x",
            "terminal_title":"merlinlabs","terminal_title_stripped":"merlinlabs",
            "workspace_id":"w1"}],"type":"agent_list"}}"#;
        let envelope: AgentListEnvelope = serde_json::from_str(json).unwrap();
        assert_eq!(envelope.result.agents.len(), 1);
        assert_eq!(envelope.result.agents[0].agent, "claude");
        assert_eq!(envelope.result.agents[0].pane_id, "w1:p1");
    }

    #[test]
    fn herdr_candidates_includes_bare_and_local_bin() {
        let c = herdr_candidates();
        assert_eq!(c[0], "herdr");
        assert!(c.iter().any(|p| p.ends_with(".local/bin/herdr")));
        assert!(!c.iter().any(|p| p.contains(".aria/bin")));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p edgeplaned-runtimes herdr_session`
Expected: FAIL — module `herdr_session` does not exist yet.

- [ ] **Step 3: Write the full implementation**

```rust
//! Subprocess wrappers around `herdr` for HerdrHosted agents.
//!
//! Mirrors zellij_session.rs's structure and testability pattern (every
//! method that shells out has a separate `*_argv` helper, unit-testable
//! without a subprocess). Two load-bearing differences from Zellij:
//!
//! - Session targeting is via the `HERDR_SESSION` environment variable, not
//!   a `--session` CLI flag.
//! - `agent prompt --wait` is an atomic submit + lifecycle-state wait
//!   (idle/working/blocked/done, with an explicit `agent_prompt_stalled`
//!   failure signal) — not a blind paste+sleep+Enter. This is strictly
//!   better than Zellij's approach and needs no plugin-routing complexity
//!   to approximate it (see zellij_hosted.rs's PluginRouting — there is no
//!   Herdr equivalent because none is needed).

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::process::Command;
use std::sync::OnceLock;

/// Fallback pane id for a freshly-created Herdr session's first pane. Real
/// dispatch discovers the pane via `discover_pane_id` — this is only the
/// fallback when discovery fails (mirrors ariad's HerdrDispatcher in the
/// `aria-rs` repo, which established this same pattern).
pub const DEFAULT_PANE_ID: &str = "w1:p1";

pub struct HerdrSession {
    pub name: String,
}

impl HerdrSession {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    /// Check whether the session is currently running on this node.
    /// `herdr session list --json` is serverless/global — works with no
    /// server running and needs no `HERDR_SESSION`.
    pub fn is_alive(&self) -> bool {
        let out = match Command::new(herdr_binary())
            .args(["session", "list", "--json"])
            .output()
        {
            Ok(o) if o.status.success() => o,
            _ => return false,
        };
        let parsed: SessionListOutput = match serde_json::from_slice(&out.stdout) {
            Ok(p) => p,
            Err(_) => return false,
        };
        parsed.sessions.iter().any(|s| s.name == self.name && s.running)
    }

    /// Discover the pane_id of the agent named "claude" in this session.
    /// Falls back to DEFAULT_PANE_ID with a loud warning if detection fails
    /// — never silently guesses wrong without saying so.
    pub fn discover_pane_id(&self) -> String {
        match self.agent_list() {
            Ok(agents) => match agents.iter().find(|a| a.agent == "claude") {
                Some(a) => a.pane_id.clone(),
                None => {
                    tracing::warn!(
                        "HerdrSession({}): no 'claude' agent found in `agent list`; \
                         falling back to {DEFAULT_PANE_ID}",
                        self.name
                    );
                    DEFAULT_PANE_ID.to_string()
                }
            },
            Err(e) => {
                tracing::warn!(
                    "HerdrSession({}): agent list failed ({e:#}); falling back to {DEFAULT_PANE_ID}",
                    self.name
                );
                DEFAULT_PANE_ID.to_string()
            }
        }
    }

    fn agent_list(&self) -> Result<Vec<AgentListEntry>> {
        let out = self
            .build_cmd(&["agent".to_string(), "list".to_string()])
            .output()
            .with_context(|| format!("herdr agent list failed for session {}", self.name))?;
        anyhow::ensure!(out.status.success(), "herdr agent list exited with {}", out.status);
        let envelope: AgentListEnvelope = serde_json::from_slice(&out.stdout)
            .with_context(|| format!("parsing agent list JSON for session {}", self.name))?;
        Ok(envelope.result.agents)
    }

    /// Deliver a prompt: atomic submit + lifecycle-state wait. Distinguishes
    /// a stalled submission from other failures so callers can tell "the
    /// agent never acknowledged" from "herdr itself errored."
    pub fn send_prompt(&self, pane_id: &str, text: &str) -> Result<()> {
        let argv = self.prompt_argv(pane_id, text);
        let out = self
            .build_cmd(&argv)
            .output()
            .with_context(|| format!("herdr agent prompt failed for session {}", self.name))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if stderr.contains("agent_prompt_stalled") {
                bail!(
                    "herdr agent prompt stalled for session {} pane {pane_id}: {stderr}",
                    self.name
                );
            }
            bail!(
                "herdr agent prompt exited {} for session {}: {stderr}",
                out.status,
                self.name
            );
        }
        Ok(())
    }

    pub fn send_keys(&self, pane_id: &str, keys: &[&str]) -> Result<()> {
        let argv = self.send_keys_argv(pane_id, keys);
        let status = self
            .build_cmd(&argv)
            .status()
            .with_context(|| format!("herdr pane send-keys failed for session {}", self.name))?;
        anyhow::ensure!(status.success(), "herdr pane send-keys exited with {status}");
        Ok(())
    }

    /// Read recent pane output. `lines` mirrors herdr's own `--lines <N>`
    /// flag — herdr has no `--full` scrollback flag like Zellij's
    /// `dump-screen --full`, so callers that want "as much as possible"
    /// should pass a large explicit value.
    pub fn dump_screen(&self, pane_id: &str, lines: Option<u32>) -> Result<String> {
        let argv = self.read_argv(pane_id, lines);
        let out = self
            .build_cmd(&argv)
            .output()
            .with_context(|| format!("herdr pane read failed for session {}", self.name))?;
        anyhow::ensure!(out.status.success(), "herdr pane read exited with {}", out.status);
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    // -- argv builders (testable without subprocess; HERDR_SESSION is set by build_cmd, not argv) --

    pub fn prompt_argv(&self, pane_id: &str, text: &str) -> Vec<String> {
        vec![
            "agent".into(),
            "prompt".into(),
            pane_id.into(),
            text.into(),
            "--wait".into(),
            "--until".into(),
            "working".into(),
            "--until".into(),
            "blocked".into(),
            "--timeout".into(),
            "15000".into(),
        ]
    }

    pub fn send_keys_argv(&self, pane_id: &str, keys: &[&str]) -> Vec<String> {
        let mut v = vec!["pane".to_string(), "send-keys".to_string(), pane_id.to_string()];
        v.extend(keys.iter().map(|k| k.to_string()));
        v
    }

    pub fn read_argv(&self, pane_id: &str, lines: Option<u32>) -> Vec<String> {
        let mut v = vec![
            "pane".to_string(),
            "read".to_string(),
            pane_id.to_string(),
            "--source".to_string(),
            "recent-unwrapped".to_string(),
        ];
        if let Some(n) = lines {
            v.push("--lines".to_string());
            v.push(n.to_string());
        }
        v
    }

    fn build_cmd(&self, argv: &[String]) -> Command {
        let mut cmd = Command::new(herdr_binary());
        cmd.env("HERDR_SESSION", &self.name);
        for a in argv {
            cmd.arg(a);
        }
        cmd
    }
}

#[derive(Debug, Deserialize)]
struct SessionListOutput {
    sessions: Vec<SessionEntry>,
}

#[derive(Debug, Deserialize)]
struct SessionEntry {
    name: String,
    running: bool,
}

#[derive(Debug, Deserialize)]
struct AgentListEnvelope {
    result: AgentListResult,
}

#[derive(Debug, Deserialize)]
struct AgentListResult {
    agents: Vec<AgentListEntry>,
}

#[derive(Debug, Deserialize)]
struct AgentListEntry {
    agent: String,
    pane_id: String,
}

/// Resolve the `herdr` binary. Probes candidate paths once, caches the
/// result for the process lifetime. Falls back to bare `"herdr"` so the
/// resulting `Command` still attempts PATH lookup and surfaces a useful
/// error from the child process rather than from this function.
///
/// systemd `--user` services start with a stripped PATH — same reasoning
/// as `zellij_binary()` in zellij_session.rs.
pub fn herdr_binary() -> &'static str {
    static RESOLVED: OnceLock<String> = OnceLock::new();
    RESOLVED
        .get_or_init(|| {
            for candidate in herdr_candidates() {
                if std::process::Command::new(&candidate)
                    .arg("--version")
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
                {
                    return candidate;
                }
            }
            "herdr".to_string()
        })
        .as_str()
}

/// Candidate locations to probe, in priority order. Deliberately generic —
/// no Aria-specific paths (e.g. `~/.aria/bin`). If a deployment pins a
/// specific herdr binary, it does so by putting that path first in the
/// *systemd unit's* PATH, the same way Aria's own fleet units pin theirs;
/// this function has no opinion about that.
pub fn herdr_candidates() -> Vec<String> {
    let mut out = vec!["herdr".to_string()];
    if let Some(home) = dirs::home_dir() {
        out.push(home.join(".cargo/bin/herdr").to_string_lossy().into_owned());
        out.push(home.join(".local/bin/herdr").to_string_lossy().into_owned());
    }
    out.push("/usr/local/bin/herdr".into());
    out.push("/usr/bin/herdr".into());
    out
}
```

(Test module from Step 1 goes at the bottom of the same file, as usual.)

- [ ] **Step 4: Wire the module and check the `serde_json` dependency**

```bash
grep -q serde_json crates/edgeplaned/crates/edgeplaned-runtimes/Cargo.toml || \
  echo 'serde_json needs to be added as a dependency to edgeplaned-runtimes/Cargo.toml (workspace version)'
```

If missing, add it under `[dependencies]` matching the workspace version used elsewhere (check `crates/edgeplaned/Cargo.toml` or another crate's `Cargo.toml` for the exact version pin already in use).

In `crates/edgeplaned/crates/edgeplaned-runtimes/src/lib.rs`, add:

```rust
pub mod herdr_session;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p edgeplaned-runtimes herdr_session`
Expected: PASS, 7/7.

- [ ] **Step 6: Commit**

```bash
git add crates/edgeplaned/crates/edgeplaned-runtimes/src/herdr_session.rs \
        crates/edgeplaned/crates/edgeplaned-runtimes/src/lib.rs \
        crates/edgeplaned/crates/edgeplaned-runtimes/Cargo.toml
git commit -m "feat(edgeplaned-runtimes): add herdr_session subprocess wrapper"
```

---

### Task 3: `herdr_hosted.rs` — the `AgentRuntime` implementation

**Files:**
- Create: `crates/edgeplaned/crates/edgeplaned-runtimes/src/herdr_hosted.rs`
- Modify: `crates/edgeplaned/crates/edgeplaned-runtimes/src/lib.rs` (add `pub mod herdr_hosted;`)

**Interfaces:**
- Consumes: `herdr_session::{HerdrSession, herdr_candidates}` (Task 2); `edgeplaned_core::agent_runtime::AgentRuntime`, `edgeplaned_core::types::{AgentHandle, AgentSignal, Capability, LaunchContext, PtySession, RuntimeKind, TaskResult, TaskSpec}` (Task 1 for `RuntimeKind::HerdrHosted` + `LaunchContext.herdr_session`); `crate::shared::merge_capabilities`.
- Produces: `pub struct HerdrHostedRuntime` implementing `AgentRuntime`, with `new()` and `with_extra_capabilities(Vec<Capability>)` constructors — same public shape as `ZellijHostedRuntime` so `daemon.rs` (Task 6) can instantiate it identically.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_is_herdr_hosted() {
        let rt = HerdrHostedRuntime::new();
        assert_eq!(rt.kind(), RuntimeKind::HerdrHosted);
    }

    #[test]
    fn capabilities_include_herdr_hosted_marker() {
        let rt = HerdrHostedRuntime::new();
        let names: Vec<&str> = rt.capabilities().iter().map(|c| c.0.as_str()).collect();
        assert!(names.contains(&"herdr_hosted"));
    }

    #[tokio::test]
    async fn launch_without_herdr_session_bails() {
        let rt = HerdrHostedRuntime::new();
        let ctx = LaunchContext { agent_id: "test".into(), ..Default::default() };
        match rt.launch(ctx).await {
            Ok(_) => panic!("launch should bail when herdr_session is None"),
            Err(e) => {
                let msg = format!("{e}");
                assert!(msg.contains("no herdr_session"), "msg: {msg}");
                assert!(msg.contains("agent_launch_context"), "msg: {msg}");
            }
        }
    }

    #[tokio::test]
    async fn signal_before_launch_bails() {
        let rt = HerdrHostedRuntime::new();
        let handle = AgentHandle {
            agent_id: "test".into(),
            runtime_kind: RuntimeKind::HerdrHosted,
            pid: 0,
        };
        let err = rt
            .signal(&handle, AgentSignal::UserInput { text: "hi".into() })
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("not registered"));
    }

    #[tokio::test]
    async fn inject_task_bails_with_routing_hint() {
        let rt = HerdrHostedRuntime::new();
        let handle = AgentHandle {
            agent_id: "test".into(),
            runtime_kind: RuntimeKind::HerdrHosted,
            pid: 0,
        };
        let task = TaskSpec {
            id: "t".into(), mission_id: "".into(), domain_id: "".into(),
            title: "".into(), description: "".into(), input_json: "{}".into(),
            required_capabilities: vec![], produces: serde_json::Value::Null,
            consumes: serde_json::Value::Null, agent_profile: None,
            domain_roster: vec![], dependency_results: vec![], pending_messages: vec![],
        };
        let err = rt.inject_task(&handle, &task).await.unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("persistent-only"));
        assert!(msg.contains("UserInput"));
    }

    #[tokio::test]
    async fn attach_pty_bails_with_routing_hint() {
        let rt = HerdrHostedRuntime::new();
        let handle = AgentHandle {
            agent_id: "test".into(),
            runtime_kind: RuntimeKind::HerdrHosted,
            pid: 0,
        };
        let err = rt.attach_pty(&handle).await.unwrap_err();
        assert!(format!("{err}").contains("edgeplane agent attach"));
    }

    #[tokio::test]
    async fn shutdown_clears_session_entry() {
        let rt = HerdrHostedRuntime::new();
        rt.sessions.lock().await.insert(
            "test".into(),
            AgentSession {
                herdr_session: "test-session".into(),
                mutex: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            },
        );
        let handle = AgentHandle {
            agent_id: "test".into(),
            runtime_kind: RuntimeKind::HerdrHosted,
            pid: 0,
        };
        rt.shutdown(handle).await.unwrap();
        assert!(!rt.sessions.lock().await.contains_key("test"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p edgeplaned-runtimes herdr_hosted`
Expected: FAIL — module doesn't exist.

- [ ] **Step 3: Write the full implementation**

```rust
//! `AgentRuntime` for agents hosted in a Herdr session.
//!
//! Mirrors zellij_hosted.rs's "thin facade" pattern exactly: this runtime
//! owns no agent process — the Herdr session is externally managed by
//! systemd (see the `aria` repo's `integrations/herdr/aria-<profile>.service`
//! units). This runtime only knows how to poke at an already-running pane.
//!
//! Unlike ZellijHostedRuntime, there is no plugin-routing path here.
//! Zellij's WASM plugin control channel exists specifically to get an
//! atomic-submit-with-completion-signal on top of a fundamentally blind
//! paste+Enter primitive. `herdr agent prompt --wait` already IS that —
//! there is nothing to route around.
//!
//! ## Attach surfaces
//!
//! `attach_pty` bails — handled outside the trait by `edgeplane agent
//! attach`, backed by `herdr_bridge.rs` (mirrors zellij_bridge.rs).
//!
//! ## Task-mode
//!
//! `inject_task` bails — HerdrHosted is persistent-only, same as ZellijHosted.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use edgeplaned_core::agent_runtime::AgentRuntime;
use edgeplaned_core::progress::ProgressEvent;
use edgeplaned_core::types::{
    AgentHandle, AgentSignal, Capability, LaunchContext, PtySession, RuntimeKind, TaskResult,
    TaskSpec,
};
use futures::stream::BoxStream;
use tokio::sync::Mutex;

use crate::herdr_session::HerdrSession;
use crate::shared::merge_capabilities;

/// Per-agent state cached at `launch()` and read at `signal()`.
struct AgentSession {
    herdr_session: String,
    mutex: Arc<Mutex<()>>,
}

pub struct HerdrHostedRuntime {
    capabilities: Vec<Capability>,
    version: String,
    sessions: Arc<Mutex<HashMap<String, AgentSession>>>,
}

impl HerdrHostedRuntime {
    pub fn new() -> Self {
        Self::with_extra_capabilities(Vec::new())
    }

    pub fn with_extra_capabilities(extra: Vec<Capability>) -> Self {
        let builtins = vec![
            Capability::new("herdr_hosted"),
            Capability::new("interactive_attach"),
            Capability::new("send_keys"),
        ];
        Self {
            capabilities: merge_capabilities(builtins, extra),
            version: "herdr_hosted 0.1 (Phase E)".into(),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for HerdrHostedRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AgentRuntime for HerdrHostedRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::HerdrHosted
    }

    fn version(&self) -> &str {
        &self.version
    }

    fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    /// Verify the externally-managed Herdr session is reachable and
    /// register the agent in the per-agent session map. Does NOT spawn
    /// anything — lifecycle is owned by systemd.
    async fn launch(&self, ctx: LaunchContext) -> Result<AgentHandle> {
        let herdr_session = ctx.herdr_session.clone().ok_or_else(|| {
            anyhow!(
                "HerdrHostedRuntime::launch: agent {} has no herdr_session in \
                 LaunchContext. This means the daemon did not resolve \
                 AgentLaunchContext.herdr_session before calling launch — \
                 check that the agent has an `agent_launch_context` row in the registry.",
                ctx.agent_id
            )
        })?;

        let session = HerdrSession::new(&herdr_session);
        if !session.is_alive() {
            tracing::warn!(
                "HerdrHostedRuntime::launch: agent {} bound to Herdr session '{}', \
                 but the session is not currently running. signal() calls will fail \
                 until the session comes back up.",
                ctx.agent_id,
                herdr_session
            );
        } else {
            tracing::info!(
                "HerdrHostedRuntime::launch: agent {} attached to live Herdr session '{}'",
                ctx.agent_id,
                herdr_session
            );
        }

        self.sessions.lock().await.insert(
            ctx.agent_id.clone(),
            AgentSession { herdr_session, mutex: Arc::new(Mutex::new(())) },
        );

        Ok(AgentHandle { agent_id: ctx.agent_id, runtime_kind: RuntimeKind::HerdrHosted, pid: 0 })
    }

    async fn inject_task(
        &self,
        _handle: &AgentHandle,
        _task: &TaskSpec,
    ) -> Result<BoxStream<'static, ProgressEvent>> {
        bail!(
            "HerdrHostedRuntime is persistent-only; inject_task is not supported. \
             Use `edgeplane agent signal` (AgentSignal::UserInput) to deliver a prompt."
        )
    }

    /// Deliver a signal to the agent's Herdr pane. Per-agent mutex
    /// serialises concurrent calls so two signals can't interleave.
    async fn signal(&self, handle: &AgentHandle, signal: AgentSignal) -> Result<()> {
        let (herdr_session, mutex) = {
            let map = self.sessions.lock().await;
            let agent = map.get(&handle.agent_id).ok_or_else(|| {
                anyhow!(
                    "HerdrHostedRuntime::signal: agent {} not registered. \
                     Was launch() called on this agent?",
                    handle.agent_id
                )
            })?;
            (agent.herdr_session.clone(), agent.mutex.clone())
        };
        let _guard = mutex.lock().await;

        let session = HerdrSession::new(&herdr_session);
        let pane_id = session.discover_pane_id();

        match signal {
            AgentSignal::UserInput { text } => session.send_prompt(&pane_id, &text),
            AgentSignal::PeerMessage { from_agent_id, body, .. } => {
                let body_text = match &body {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                let formatted = format!("[from {from_agent_id}]: {body_text}");
                session.send_prompt(&pane_id, &formatted)
            }
            AgentSignal::Cancel => session.send_keys(&pane_id, &["ctrl+c"]),
        }
    }

    async fn collect_result(&self, handle: &AgentHandle) -> Result<TaskResult> {
        Ok(TaskResult {
            task_id: handle.agent_id.clone(),
            success: true,
            exit_code: 0,
            artifact_path: None,
            summary: "HerdrHosted has no task lifecycle".into(),
        })
    }

    async fn attach_pty(&self, _handle: &AgentHandle) -> Result<PtySession> {
        bail!(
            "HerdrHostedRuntime does not support PTY attach via this trait method; \
             use `edgeplane agent attach <id>` (execs `herdr session attach`) instead"
        )
    }

    async fn shutdown(&self, handle: AgentHandle) -> Result<()> {
        tracing::debug!(
            "HerdrHostedRuntime::shutdown: removing agent {} from session map \
             (no process to terminate — Herdr session is externally managed)",
            handle.agent_id
        );
        self.sessions.lock().await.remove(&handle.agent_id);
        Ok(())
    }

    async fn ensure_installed(&self) -> Result<()> {
        for candidate in crate::herdr_session::herdr_candidates() {
            let probe = tokio::process::Command::new(&candidate).arg("--version").output().await;
            if let Ok(out) = probe
                && out.status.success()
            {
                tracing::debug!("HerdrHostedRuntime: found herdr at {}", candidate);
                return Ok(());
            }
        }
        bail!(
            "herdr binary not found in any of the candidate locations \
             (PATH, ~/.cargo/bin, ~/.local/bin, /usr/local/bin, /usr/bin)."
        )
    }
}
```

(Test module from Step 1 goes at the bottom.)

- [ ] **Step 4: Wire the module**

In `lib.rs`: `pub mod herdr_hosted;`

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p edgeplaned-runtimes herdr_hosted`
Expected: PASS, 6/6.

- [ ] **Step 6: Commit**

```bash
git add crates/edgeplaned/crates/edgeplaned-runtimes/src/herdr_hosted.rs \
        crates/edgeplaned/crates/edgeplaned-runtimes/src/lib.rs
git commit -m "feat(edgeplaned-runtimes): add HerdrHostedRuntime AgentRuntime impl"
```

---

### Task 4: Registry schema — `herdr_session` column (both mirrored files)

**Files:**
- Modify: `crates/edgeplaned/crates/edgeplaned-bin/src/local_registry.rs` (daemon-side, versioned migration)
- Modify: `crates/edgeplane/src/local_db.rs` (CLI-side, schema literal + `ManifestProfile` + `import_manifest`)

**Interfaces:**
- Produces: `agent_launch_context.herdr_session TEXT` column in both databases' schema (must match exactly — same table, same column name, since both processes open the same file); `AgentLaunchContext.herdr_session: Option<String>` (local_registry.rs); `ManifestProfile.herdr_session: Option<String>` + `import_manifest()` populates it when `runtime == "herdr_hosted"` (local_db.rs).

- [ ] **Step 1: Write the failing test (local_registry.rs)**

Add near the existing launch-context tests in `local_registry.rs`:

```rust
#[test]
fn launch_context_roundtrips_herdr_session() {
    let dir = tempfile::TempDir::new().unwrap();
    let registry = LocalRegistry::open(&dir.path().join("registry.db")).unwrap();
    let record = AgentRecord {
        id: "vega".into(), source: "test".into(), domain_id: "".into(),
        runtime_kind: "herdr_hosted".into(), supervision_mode: "persistent".into(),
        capabilities_json: "[]".into(), profile_path: None,
        enrolled_at: "2026-08-19T00:00:00Z".into(), last_synced_at: None,
    };
    registry.upsert(&record).unwrap();
    let ctx = AgentLaunchContext {
        source: "test".into(), agent_id: "vega".into(),
        vault_folder: None, state_dir_spec: None,
        zellij_session: None, herdr_session: Some("vega".into()),
        systemd_service: None, supervise_paused: false,
    };
    registry.upsert_launch_context(&ctx).unwrap();
    let round_tripped = registry.get_launch_context("test", "vega").unwrap().unwrap();
    assert_eq!(round_tripped.herdr_session.as_deref(), Some("vega"));
    assert_eq!(round_tripped.zellij_session, None);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p edgeplaned-bin launch_context_roundtrips_herdr_session`
Expected: FAIL to compile — `AgentLaunchContext` has no field `herdr_session`.

- [ ] **Step 3: Add the v3 migration and field (local_registry.rs)**

Bump `CURRENT_SCHEMA_VERSION` to `3` and update the version-map comment:

```rust
//   v3 → Phase E: agent_launch_context gains herdr_session column.
const CURRENT_SCHEMA_VERSION: u32 = 3;
```

In `apply_migrations`, after the `v2` block:

```rust
    if current < 3 {
        migrate_to_v3(conn).context("schema migration to v3")?;
    }
```

Add the migration function next to `migrate_to_v2`:

```rust
fn migrate_to_v3(conn: &Connection) -> Result<()> {
    add_column_if_missing(conn, "agent_launch_context", "herdr_session", "TEXT")
}
```

Add the field to `AgentLaunchContext` right after `zellij_session`:

```rust
    pub herdr_session: Option<String>,
```

Update `row_to_launch_context` to read it — check the column-index comments carefully, this is an `ALTER TABLE ADD COLUMN`, so it lands as the *last* column in `PRAGMA table_info` order, not adjacent to `zellij_session`. Find the existing `SELECT` statement(s) that build the row tuple (search `upsert_launch_context`'s `INSERT INTO agent_launch_context` column list and any `SELECT ... FROM agent_launch_context` in `get_launch_context`/`list_all_launch_contexts`) and add `herdr_session` to both the column list and the `row.get(N)?` call at the correct new index — **read the actual current SELECT column order before writing the index**, don't assume it matches the CREATE TABLE order.

Also update `upsert_launch_context`'s `INSERT ... ON CONFLICT DO UPDATE` to include `herdr_session` in both the column list, the `VALUES` placeholder list, and the `DO UPDATE SET` clause (mirroring exactly how `zellij_session` is already handled there).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p edgeplaned-bin launch_context_roundtrips_herdr_session`
Expected: PASS.

- [ ] **Step 5: Mirror the same column in `local_db.rs` (CLI-side)**

This is a **separate, independently-maintained schema literal** — SQLite's `ALTER TABLE ADD COLUMN` via `local_registry.rs`'s migration only fixes up databases opened by the *daemon*. If the CLI opens a **fresh** database first (no daemon has run yet), `local_db.rs::ensure_schema()`'s `CREATE TABLE IF NOT EXISTS` must already include the column, or that fresh DB silently lacks it forever (its `IF NOT EXISTS` guard means it will never retroactively add columns).

In `ensure_schema()`'s `CREATE TABLE agent_launch_context` literal, add:

```rust
             herdr_session   TEXT,
```

(right after the `zellij_session TEXT,` line).

Add a test proving a fresh DB has the column:

```rust
#[test]
fn fresh_db_has_herdr_session_column() {
    let dir = tempfile::TempDir::new().unwrap();
    let conn = edgeplaned_paths::open_tuned(&dir.path().join("registry.db")).unwrap();
    ensure_schema(&conn).unwrap();
    let mut stmt = conn.prepare("PRAGMA table_info(agent_launch_context)").unwrap();
    let cols: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert!(cols.contains(&"herdr_session".to_string()), "columns: {cols:?}");
}
```

Run: `cargo test -p edgeplane fresh_db_has_herdr_session_column` — expect FAIL first (column missing), then PASS after the edit.

- [ ] **Step 6: `ManifestProfile` + `import_manifest()` gain `herdr_session` (local_db.rs)**

Add to `ManifestProfile`:

```rust
    /// Only required for `herdr_hosted` runtime. Optional so profiles on
    /// other runtimes can omit it.
    pub herdr_session: Option<String>,
```

In `import_manifest()`, generalize the `is_acp` exclusivity check — currently `zellij_session` is nulled for ACP profiles; now `herdr_session` needs the mirror-image treatment (nulled unless `runtime_kind == "herdr_hosted"`, and `zellij_session` should also be nulled when the runtime IS herdr_hosted, since a profile shouldn't carry both):

```rust
        let is_acp = runtime_kind == "claude_agent_acp";
        let is_herdr = runtime_kind == "herdr_hosted";
        // ...
        let zellij_session: Option<&str> =
            if is_acp || is_herdr { None } else { profile.zellij_session.as_deref() };
        let herdr_session: Option<&str> =
            if is_herdr { profile.herdr_session.as_deref() } else { None };
```

Update the `INSERT INTO agent_launch_context` statement to include `herdr_session` in its column list, placeholders, and `ON CONFLICT DO UPDATE SET` clause, and pass `herdr_session` in the `params![...]` call alongside the existing `zellij_session`.

Add a test mirroring `import_acp_profile_sets_runtime_and_profile_path`:

```rust
#[test]
fn import_herdr_profile_sets_herdr_session_and_null_zellij() {
    let dir = TempDir::new().unwrap();
    let registry_path = dir.path().join("registry.db");
    // import_manifest opens its own connection via db_path()/open() internally
    // in the real function — this test needs whatever seam the existing
    // ACP-profile test already uses to redirect that path (check
    // `import_acp_profile_sets_runtime_and_profile_path`'s setup and reuse
    // the same pattern rather than inventing a new one).
    let manifest = r#"
[[profile]]
name           = "vega"
herdr_session  = "vega"
service        = "aria-vega.service"
state_dir      = "/tmp/test-profiles/vega"
runtime        = "herdr_hosted"
"#;
    let path = dir.path().join("fleet.toml");
    std::fs::write(&path, manifest).unwrap();
    let summary = import_manifest(&path, "test").unwrap();
    assert_eq!(summary.created, 1);
    // ... assert on the row: runtime_kind == "herdr_hosted", herdr_session
    // == Some("vega"), zellij_session == None. Match the exact read pattern
    // the existing tests in this file already use.
}
```

**Note for the implementer:** the existing tests in this file call `import_manifest(&path, source)` directly without visibly redirecting `db_path()` — read how `import_manifest_creates_agents_and_contexts` (or equivalent) actually isolates its database before writing this test; do not guess a mocking seam that doesn't exist.

- [ ] **Step 7: Run all tests in both crates**

Run: `cargo test -p edgeplaned-bin -p edgeplane`
Expected: all pass, including the new ones.

- [ ] **Step 8: Commit**

```bash
git add crates/edgeplaned/crates/edgeplaned-bin/src/local_registry.rs \
        crates/edgeplane/src/local_db.rs
git commit -m "feat(registry): add herdr_session column to agent_launch_context (both mirrored schemas)"
```

---

### Task 5: `SpawnOverrides.herdr_session` + wire both `daemon.rs` construction sites

**Files:**
- Modify: `crates/edgeplaned/crates/edgeplaned-bin/src/supervisor.rs`
- Modify: `crates/edgeplaned/crates/edgeplaned-bin/src/daemon.rs`

**Interfaces:**
- Consumes: `AgentLaunchContext.herdr_session` (Task 4), `LaunchContext.herdr_session` (Task 1).
- Produces: `SpawnOverrides.herdr_session: Option<String>`, correctly populated from the registry at both call sites, flowing into `LaunchContext.herdr_session` at spawn time.

- [ ] **Step 1: Add the field to `SpawnOverrides` (supervisor.rs)**

```rust
    pub herdr_session: Option<String>,
```

Add it to the `LaunchContext { .. }` construction in `Supervisor::spawn`:

```rust
            herdr_session: overrides.herdr_session,
```

- [ ] **Step 2: Write a failing test for the federated-merge path**

Find the existing test(s) around `daemon.rs`'s federated-merge logic (search for a test that exercises the code block containing `spec.launch_overrides = crate::supervisor::SpawnOverrides { ... zellij_session: matched_ctx.zellij_session.clone() ...}` around line 1209 — there should be one given `reconcile.rs` has extensive coverage of this area per the earlier grep). Add a case: a local context with `herdr_session: Some("vega".into())` and `zellij_session: None`, assert the merged spec's `launch_overrides.herdr_session == Some("vega".into())`.

- [ ] **Step 3: Run to verify it fails**

Expect a compile error (`SpawnOverrides` has no field `herdr_session` yet in the test) or an assertion failure once Step 1 is done but before Step 4 — do Step 1 first as noted above, then this test should fail only on the assertion (daemon.rs not yet populating it), confirming the gap precisely.

- [ ] **Step 4: Populate `herdr_session` at both construction sites (daemon.rs)**

Site 1 (~line 1209, federated-merge path):

```rust
        spec.launch_overrides = crate::supervisor::SpawnOverrides {
            vault_folder: matched_ctx.vault_folder.clone(),
            state_dir_spec: matched_ctx.state_dir_spec.clone(),
            zellij_session: matched_ctx.zellij_session.clone(),
            herdr_session: matched_ctx.herdr_session.clone(),
        };
```

Also extend the adjacent `tracing::info!` log line to include `herdr_session={:?}` alongside `zellij_session={:?}` — this log line is exactly what confirmed today, live, that the *old* zellij-only version of this merge was working; keep that debuggability for the new field.

Site 2 (~line 1672, standalone-listing path):

```rust
                                spec.launch_overrides = SpawnOverrides {
                                    vault_folder: ctx.vault_folder,
                                    state_dir_spec: ctx.state_dir_spec,
                                    zellij_session: ctx.zellij_session,
                                    herdr_session: ctx.herdr_session,
                                };
```

- [ ] **Step 5: Fix any other `SpawnOverrides { .. }` literal missing the new field**

`cargo build -p edgeplaned-bin 2>&1 | grep "missing field \`herdr_session\`"` — fix each (expect test-fixture constructors in `daemon.rs`'s and `reconcile.rs`'s test modules; add `herdr_session: None,` unless the specific test is about Herdr).

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p edgeplaned-bin`
Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add crates/edgeplaned/crates/edgeplaned-bin/src/supervisor.rs \
        crates/edgeplaned/crates/edgeplaned-bin/src/daemon.rs
git commit -m "feat(edgeplaned-bin): wire herdr_session through SpawnOverrides at both registry merge sites"
```

---

### Task 6: Register `HerdrHostedRuntime` in the runtime-instantiation match

**Files:**
- Modify: `crates/edgeplaned/crates/edgeplaned-bin/src/daemon.rs`

**Interfaces:**
- Consumes: `HerdrHostedRuntime` (Task 3).
- Produces: `runtime_kind == "herdr_hosted"` now resolves to a real runtime instead of hitting the `other => { warn!("Unknown runtime kind..."); return None }` fallback.

- [ ] **Step 1: Write a failing test**

Find the existing test that exercises this match arm for `"zellij_hosted"` (search near the match block for a test name like `resolves_zellij_hosted_runtime` or similar — the match is at the site with `"zellij_hosted" => Arc::new(Box::new(ZellijHostedRuntime::with_extra_capabilities(extra_caps)))`). Add the mirror case for `"herdr_hosted"`, asserting the resolved runtime's `.kind() == RuntimeKind::HerdrHosted`.

- [ ] **Step 2: Run to verify it fails**

Expected: the new test fails — `"herdr_hosted"` currently falls into the `other =>` arm and returns `None`.

- [ ] **Step 3: Add the match arm**

```rust
            "herdr_hosted" => Arc::new(Box::new(
                HerdrHostedRuntime::with_extra_capabilities(extra_caps),
            )),
```

right after the existing `"zellij_hosted"` arm. Add the import at the top of the file alongside the existing `zellij_hosted::ZellijHostedRuntime` import:

```rust
    herdr_hosted::HerdrHostedRuntime,
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p edgeplaned-bin`
Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add crates/edgeplaned/crates/edgeplaned-bin/src/daemon.rs
git commit -m "feat(edgeplaned-bin): register herdr_hosted in the runtime-instantiation match"
```

---

### Task 7: PTY bridge — `herdr_bridge.rs` + wire into the persistent-mode branch

**Files:**
- Create: `crates/edgeplaned/crates/edgeplaned-bin/src/herdr_bridge.rs`
- Modify: `crates/edgeplaned/crates/edgeplaned-bin/src/daemon.rs` (persistent-mode branch, and `mod herdr_bridge;` in `main.rs` or wherever `mod zellij_bridge;` currently lives)

**Interfaces:**
- Consumes: `AttachRegistry`/`AttachEndpoints`/`PtyAttachEndpoints` (existing, from `attach_registry.rs` — read that file's public surface before writing this task's code; not re-derived here since it's unchanged from what `zellij_bridge.rs` already consumes).
- Produces: `pub async fn run_for_agent(agent_id: String, herdr_session: String, registry: Arc<AttachRegistry>)`, same signature shape as `zellij_bridge::run_for_agent`.

- [ ] **Step 1: Read `attach_registry.rs`'s relevant public types**

Before writing this file, run: `grep -n "pub struct AttachEndpoints\|pub struct PtyAttachEndpoints\|impl AttachRegistry" crates/edgeplaned/crates/edgeplaned-bin/src/attach_registry.rs` and read enough of that file to know the exact registration call `zellij_bridge.rs` makes (`registry.register(...)` or similar) — reuse it identically, this bridge only changes what's spawned, not how it registers.

- [ ] **Step 2: Write the implementation** (adapted from `zellij_bridge.rs`, which is the direct template — same backoff/restart scaffolding, same `portable_pty` usage; only the spawned command and its argv change)

```rust
//! PTY bridge for HerdrHosted agents.
//!
//! Spawns `herdr session attach <session_name>` as a PTY child and
//! registers `PtyAttachEndpoints` in the `AttachRegistry`, enabling remote
//! terminal viewing through the existing `attach_ws` → `pump_pty` pipeline.
//! Directly mirrors zellij_bridge.rs — same backoff/restart scaffolding,
//! only the spawned command differs.
//!
//! The bridge does NOT own the Herdr session — systemd services manage the
//! session lifecycle. This module only provides a PTY view into it.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use edgeplaned_core::types::AgentSignal;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use tokio::sync::{broadcast, mpsc};

use crate::attach_registry::{AttachEndpoints, AttachRegistry, PtyAttachEndpoints};

const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(60);
const STABLE_THRESHOLD: Duration = Duration::from_secs(30);
const STDOUT_BROADCAST_CAPACITY: usize = 1024;
const DEFAULT_ROWS: u16 = 50;
const DEFAULT_COLS: u16 = 220;

pub async fn run_for_agent(agent_id: String, herdr_session: String, registry: Arc<AttachRegistry>) {
    let mut backoff = BACKOFF_MIN;

    loop {
        let started = Instant::now();
        match run_one_bridge(&agent_id, &herdr_session, &registry).await {
            Ok(()) => {
                tracing::info!(
                    "Herdr bridge for agent {agent_id} (session {herdr_session}) \
                     exited cleanly after {:?}",
                    started.elapsed()
                );
            }
            Err(e) => {
                tracing::warn!(
                    "Herdr bridge for agent {agent_id} (session {herdr_session}) \
                     failed after {:?}: {e:#}",
                    started.elapsed()
                );
            }
        }

        if started.elapsed() >= STABLE_THRESHOLD {
            backoff = BACKOFF_MIN;
        }

        tracing::info!("Restarting Herdr bridge for {agent_id} in {backoff:?}");
        tokio::time::sleep(backoff).await;
        backoff = std::cmp::min(backoff * 2, BACKOFF_MAX);
    }
}
```

**Note for the implementer:** the rest of `run_one_bridge` (PTY spawn, stdout broadcast fan-out, signal-to-stdin rendering, `AttachEndpoints` registration/deregistration) should be copied from `zellij_bridge.rs`'s own `run_one_bridge` with exactly one substitution: the `CommandBuilder` target changes from

```rust
let mut cmd = CommandBuilder::new(zellij_bin);
cmd.arg("attach");
cmd.arg(zellij_session);
```

to

```rust
let mut cmd = CommandBuilder::new(edgeplaned_runtimes::herdr_session::herdr_binary());
cmd.arg("session");
cmd.arg("attach");
cmd.arg(herdr_session);
```

Read the full body of `zellij_bridge.rs::run_one_bridge` (it continues past the excerpt already read in this session — `wc -l` showed 269 lines total, only the first ~60 were read) before writing this task's code, and copy it verbatim except for that one substitution and the renamed parameter (`zellij_session: &str` → `herdr_session: &str`).

- [ ] **Step 3: Wire the module**

Find `mod zellij_bridge;` (likely in `main.rs`) and add `mod herdr_bridge;` next to it.

- [ ] **Step 4: Wire the persistent-mode branch in `daemon.rs`**

Find the block:

```rust
                if spec.runtime_kind == "zellij_hosted" {
                    if let Some(zellij_session) = spec.launch_overrides.zellij_session.clone() {
                        let bridge_jh = tokio::spawn(crate::zellij_bridge::run_for_agent(
                            spec.agent_id.clone(),
                            zellij_session.clone(),
                            self.attach_registry.clone(),
                        ));
                        // ...
```

Add a parallel arm right after its closing brace:

```rust
                if spec.runtime_kind == "herdr_hosted" {
                    if let Some(herdr_session) = spec.launch_overrides.herdr_session.clone() {
                        let bridge_jh = tokio::spawn(crate::herdr_bridge::run_for_agent(
                            spec.agent_id.clone(),
                            herdr_session.clone(),
                            self.attach_registry.clone(),
                        ));
                        handles.push(bridge_jh);
                        tracing::info!(
                            "HerdrHosted agent {} registered with PTY bridge \
                             (session '{herdr_session}')",
                            spec.agent_id
                        );
                    } else {
                        tracing::info!(
                            "HerdrHosted agent {} registered without PTY bridge \
                             (no herdr_session in launch_overrides)",
                            spec.agent_id
                        );
                    }
                    return Some(RunningAgent::new(spec.clone(), handles));
                }
```

- [ ] **Step 5: Build and run tests**

Run: `cargo build -p edgeplaned-bin && cargo test -p edgeplaned-bin`
Expected: clean build (this task is mostly mechanical port — no meaningfully new unit-testable logic beyond what Task 2/3 already covered; the real verification is Task 9's live smoke test).

- [ ] **Step 6: Commit**

```bash
git add crates/edgeplaned/crates/edgeplaned-bin/src/herdr_bridge.rs \
        crates/edgeplaned/crates/edgeplaned-bin/src/daemon.rs \
        crates/edgeplaned/crates/edgeplaned-bin/src/main.rs
git commit -m "feat(edgeplaned-bin): add Herdr PTY bridge, wire into persistent-mode spawn"
```

---

### Task 8: Registry visibility — `mgmt_gateway.rs` + `agent_ops.rs` gain `herdr_session`

**Files:**
- Modify: `crates/edgeplaned/crates/edgeplaned-bin/src/mgmt_gateway.rs`
- Modify: `crates/edgeplane/src/agent_ops.rs`

**Interfaces:**
- Consumes: `AgentLaunchContext.herdr_session` (Task 4).
- Produces: `edgeplane agent list` / `edgeplane agent describe` JSON output includes `herdr_session` alongside `zellij_session`.

- [ ] **Step 1: Add `herdr_session` to `list_local_agents`'s JSON emission**

In `mgmt_gateway.rs::list_local_agents`, both `serde_json::json!({...})` blocks (context-backed and context-less) gain:

```rust
                "herdr_session": ctx.herdr_session,
```

(context-less block: `"herdr_session": null,`)

- [ ] **Step 2: Check `describe_local_agent` for the same gap**

Read `describe_local_agent`'s body (not yet read this session — its signature was seen but not its implementation) and add the equivalent `herdr_session` field to whatever JSON it constructs, mirroring however it currently emits `zellij_session`.

- [ ] **Step 3: Add the CLI-side describe output line**

In `crates/edgeplane/src/agent_ops.rs`, right after:

```rust
    if let Some(zs) = agent.get("zellij_session").and_then(|v| v.as_str()) {
        println!("zellij_session:{zs}");
    }
```

add:

```rust
    if let Some(hs) = agent.get("herdr_session").and_then(|v| v.as_str()) {
        println!("herdr_session:{hs}");
    }
```

Check the second `zellij_session` reference at (previously seen) line ~533 for the same pattern and mirror it there too.

- [ ] **Step 4: Build**

Run: `cargo build --workspace`
Expected: clean (this task has no new unit-testable logic — verified in Task 9's live check via `edgeplane agent describe <id>`).

- [ ] **Step 5: Commit**

```bash
git add crates/edgeplaned/crates/edgeplaned-bin/src/mgmt_gateway.rs \
        crates/edgeplane/src/agent_ops.rs
git commit -m "feat(edgeplane): surface herdr_session in agent list/describe output"
```

---

### Task 9: CLI attach support — `attach.rs`

**Files:**
- Modify: `crates/edgeplane/src/attach.rs`

**Interfaces:**
- Consumes: `herdr_session` field on the `agent.describe_local` JSON response (Task 8).
- Produces: `edgeplane agent attach <herdr-hosted-agent-id>` execs `herdr session attach <name>`; `--web` bails with a clear "herdr has no web frontend" message instead of the generic ZellijHosted-only message.

- [ ] **Step 1: Read the dispatch point that currently calls `attach_zellij_hosted`**

Find where `attach_zellij_hosted(args, info)` is called (near the `--web` bail block already read) — it's presumably gated on `info.get("runtime_kind") == Some("zellij_hosted")` or similar. Read that exact condition before writing the parallel branch.

- [ ] **Step 2: Add the `herdr_hosted` branch**

```rust
/// Attach to a HerdrHosted agent — exec `herdr session attach <session>`.
/// Unlike ZellijHosted, there is no `--web` equivalent: Herdr has no web
/// frontend (confirmed against the pinned v0.8.0 CLI surface — no `herdr
/// web` subcommand exists). Remote access for HerdrHosted agents is the
/// Claude mobile/desktop app (primary) or `herdr --remote <ssh-target>
/// --session <name>` over SSH.
fn attach_herdr_hosted(args: &AttachArgs, info: &serde_json::Value) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let session = info
        .get("herdr_session")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            anyhow!(
                "agent `{}` is HerdrHosted but has no herdr_session in its launch context",
                args.agent_id
            )
        })?;

    if args.web {
        bail!(
            "--web is not supported for HerdrHosted agents (herdr has no web frontend). \
             Use the Claude app, or `herdr --remote <ssh-target> --session {session}` over SSH."
        );
    }

    let err = std::process::Command::new(edgeplaned_runtimes::herdr_session::herdr_binary())
        .arg("session")
        .arg("attach")
        .arg(session)
        .exec();
    Err(anyhow!("exec herdr session attach failed: {err}"))
}
```

Wire it into the dispatch point found in Step 1, parallel to the existing `zellij_hosted` branch (match/if-else on `runtime_kind`).

**Note:** confirm whether `crates/edgeplane` already depends on `edgeplaned-runtimes` (check `crates/edgeplane/Cargo.toml`) — if not, either add that dependency or duplicate the small `herdr_binary()` probe locally in `attach.rs` (check how `zellij` is invoked in the neighboring `attach_zellij_hosted` — it uses a bare `"zellij"` literal, not `zellij_session::zellij_binary()`'s probing, since this is a foreground `exec` where PATH is the user's interactive shell PATH, not a stripped systemd PATH). **If the existing code uses a bare `"zellij"` literal here, follow that same precedent and use a bare `"herdr"` literal too** — don't introduce asymmetry with the probing logic that only matters for systemd-launched processes.

- [ ] **Step 3: Build**

Run: `cargo build -p edgeplane`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/edgeplane/src/attach.rs
git commit -m "feat(edgeplane): add HerdrHosted branch to CLI attach"
```

---

### Task 10: End-to-end live validation against the real Aria fleet

**Files:** none (verification only — no code changes).

**Interfaces:** consumes everything from Tasks 1–9.

- [ ] **Step 1: Full workspace gate**

```bash
cd ~/code/edgeplane
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo nextest run --workspace --locked
```
All must pass before touching the live fleet.

- [ ] **Step 2: Deploy the built binaries**

Follow this repo's own deploy convention (check for a `scripts/deploy*.sh` or equivalent — do not invent a path; if none exists, `cargo build --release` and manually install to wherever `edgeplaned.service`/`edgeplane` on excalibur currently resolve their binaries from, confirmed via `systemctl --user cat edgeplaned.service` and `which edgeplane`).

- [ ] **Step 3: Import ONE profile as `herdr_hosted` first (not all 6 at once)**

Pick a low-stakes profile — **not** `merlinlabs` or `operator`, since those are actively used; `gwyn` is a reasonable choice since she's on-demand and currently `inactive`, minimizing collision risk with a live conversation. Write a one-profile manifest:

```toml
[[profile]]
name          = "gwyn"
herdr_session = "gwyn"
service       = "aria-gwyn.service"
state_dir     = "/home/merlin/.claude/profiles/gwyn"
runtime       = "herdr_hosted"
```

```bash
edgeplane daemon agent import /path/to/one-profile.toml --source manifest_import
edgeplane agent describe gwyn   # confirm herdr_session:gwyn appears, runtime_kind:herdr_hosted
```

Restart `edgeplaned.service` (or wait for its reconcile tick — check the interval in the code/docs rather than assuming 60s) so it picks up the new registry row.

- [ ] **Step 4: Confirm the WARN-loop is gone**

```bash
journalctl --user -u edgeplaned.service --since "-2 min" --no-pager | grep -i gwyn
```
Expected: no `Zellij bridge ... failed` lines for gwyn; if the PTY bridge (Task 7) wired correctly, expect a `HerdrHosted agent ... registered with PTY bridge` info line instead.

- [ ] **Step 5: Live signal round-trip (start gwyn's session first — she's normally inactive)**

```bash
systemctl --user start aria-gwyn.service
sleep 5
edgeplane agent signal gwyn --content "Respond with exactly: EP-HERDR-SIGNAL-OK. Do nothing else."
sleep 3
HERDR_SESSION=gwyn herdr pane read w1:p1 --source recent-unwrapped | grep EP-HERDR-SIGNAL-OK
```
Expected: the marker string appears — proves `edgeplane agent signal` now reaches a Herdr pane end-to-end, through the real daemon, not just a manual `herdr` CLI call.

- [ ] **Step 6: PTY attach round-trip**

```bash
timeout 5 edgeplane agent attach gwyn
```
Expected: attaches to a live terminal view (timeout kills it after 5s since this is a scripted check, not an interactive session — confirm it didn't error out before the timeout by checking exit behavior, not by watching it manually for 5s).

- [ ] **Step 7: Regression check — durable mesh unaffected**

```bash
edgeplane agent signal gwyn --content "mesh regression check" # already covered by Step 5's success
```
And confirm `send_mesh_message`/`list_mesh_messages` (a completely different code path, untouched by this plan) still work for any other profile — this is a sanity check that nothing in Tasks 1–9 broke the unrelated mesh bus, not a new capability being tested.

- [ ] **Step 8: Stop gwyn, restoring her normal inactive state**

```bash
systemctl --user stop aria-gwyn.service
```

- [ ] **Step 9: Roll out to the remaining 5 migrated profiles**

Repeat Steps 3–4 for vega, publisher, wyatt, engineer, merlinlabs — via a full manifest import (all 6 `herdr_hosted` profiles in one file) rather than one at a time, now that gwyn's round-trip has proven the mechanism. Confirm the WARN-loop is gone fleet-wide:

```bash
journalctl --user -u edgeplaned.service --since "-2 min" --no-pager | grep -i "zellij bridge"
```
Expected: empty (no Zellij-bridge lines for any of the 6 — they should all now show `HerdrHosted agent ... registered with PTY bridge` instead).

- [ ] **Step 10: Update the aria-side record**

This plan lives in the `edgeplane` repo; the migration's own execution record lives in `aria`'s `docs/superpowers/specs/2026-08-18-herdr-fleet-migration-v2.md` §8 (Phase E) and the vault note `Aria/Operator/plans/2026-08-19-herdr-evaluation-and-phase-e-recommendation.md`. Update both to mark Phase E done, with the commit range from this plan and the live-validation evidence from Steps 4–9.

---

## Self-Review

**Spec coverage:** v1 §8's three work items are covered — item 1 (herdr_hosted.rs runtime) → Tasks 1–3; item 2 (fleet_import runtime value + re-import) → Tasks 4–6, corrected to target the *actual* live path (`local_db.rs`) instead of the dead `fleet_import.rs`; item 3 (PTY bridge decision) → Task 7, per Merlin's "keep it" call.

**Placeholder scan:** no TBD/TODO. Two spots intentionally hand real investigation work to the implementer rather than guessing (Task 7's `run_one_bridge` body copy, Task 4's exact `SELECT` column index) — both are read-the-existing-code-first instructions with a named target, not vague "add appropriate handling."

**Type consistency:** `HerdrSession`/`herdr_binary`/`herdr_candidates` (Task 2) are the names Tasks 3, 7, and 9 all consume — checked consistent throughout. `SpawnOverrides.herdr_session` (Task 5) matches `LaunchContext.herdr_session` (Task 1) and `AgentLaunchContext.herdr_session` (Task 4) in name and type (`Option<String>`) across all four files that touch it.

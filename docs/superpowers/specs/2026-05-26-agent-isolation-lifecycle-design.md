# Agent Process Isolation & Lifecycle Architecture

**Date:** 2026-05-26
**Author:** Merlin + Aria (Engineer)
**Status:** Design approved, pending implementation planning

## Problem Statement

edgeplaned manages agent processes with tight coupling and zero isolation:

1. **Lifecycle coupling:** ACP and task-mode agents are child processes with `kill_on_drop(true)`. Daemon restart kills all agents. No process adoption, no graceful drain, no state survives.

2. **No isolation:** All agents run as the same Unix user with full filesystem, network, and process visibility. One agent can read another's secrets, kill another's processes, or clobber shared state.

3. **No permission model:** Any authenticated user can deploy an unisolated agent. No linkage between user identity and isolation enforcement.

The sandbox crate (`edgeplaned-sandbox`) already implements a full Linux jail (user namespaces, mount namespace + pivot_root, Landlock, seccomp, capability drop, cgroup limits, network egress allowlists, binary hash verification) but is not wired into any spawn path.

## Design Principles

- **Incus model:** edgeplaned operates over agent runtimes, not as their parent. Agents survive daemon restarts. edgeplaned adopts running agents on startup.
- **Graduated isolation:** Three levels (`none`, `namespace`, `jailed`) chosen per-agent, enforced by role ceiling. Strictest level that satisfies the trust model wins.
- **Isolation is a ceiling, not a choice:** User role determines the maximum relaxation. Admins can opt out. Members cannot.
- **Backwards compatible:** Existing fleet profiles default to `namespace` with ability to relax to `none`. Zero behavior change without explicit opt-in.

## Architecture Overview

Three phases, each delivering standalone value:

| Phase | Delivers | Depends on |
|-------|----------|-----------|
| 1. Process Lifecycle | Daemon-independent agents, adoption on restart, graceful shutdown | Nothing |
| 2. Graduated Isolation | Namespace and jail enforcement at spawn, adversarial validation | Phase 1 (spawn path changes) |
| 3. Role-Based Permissions | OIDC-to-role mapping, isolation ceiling enforcement | Phase 2 (isolation levels exist) |

---

## Phase 1: Agent Process Lifecycle

### Goal

Agent processes survive edgeplaned restarts. edgeplaned can adopt running agents instead of spawning fresh. Graceful shutdown drains in-flight work.

### Changes

#### 1.1 Process independence via `setsid`

Replace `cmd.kill_on_drop(true)` in `edgeplaned-acp/src/agent.rs:158` with:

- Call `setsid()` before exec (via `pre_exec` hook on `tokio::process::Command`). The agent gets its own session — kernel won't deliver SIGHUP when edgeplaned dies.
- Remove `kill_on_drop(true)`. edgeplaned no longer implicitly owns the child's lifecycle.
- Explicit shutdown replaces implicit drop (see 1.4).

#### 1.2 Runtime state file

At spawn, edgeplaned writes `~/.edgeplane/agents/<agent_id>/runtime.json`:

```json
{
  "pid": 84521,
  "socket_path": "/run/user/1000/edgeplaned-agent-work.sock",
  "runtime_kind": "claude_agent_acp",
  "isolation_level": "none",
  "started_at": "2026-05-26T12:00:00Z",
  "resume_token": "acp_session_abc123",
  "owner_subject": "merlin@authentik"
}
```

This is the adoption contract. Fields:
- `pid`: OS process ID, used for liveness probe.
- `socket_path`: IPC endpoint (ACP agents) or null (Zellij-hosted).
- `runtime_kind`: Which runtime spawned this agent.
- `isolation_level`: What isolation was applied at spawn.
- `started_at`: For age-based decisions (nightly restart, stale detection).
- `resume_token`: ACP session resume token, if applicable. Enables session continuity across daemon restarts.
- `owner_subject`: OIDC subject or service account name. Used by Phase 3 for role resolution.

Zellij-hosted agents also get a state file for consistency (pid = systemd main PID, socket_path = null).

#### 1.3 Adoption on restart

During daemon startup, after `resolve_agent_specs()` produces the desired agent list:

```
for each agent_id in desired:
    state_file = ~/.edgeplane/agents/<agent_id>/runtime.json
    if state_file exists:
        if pid_is_alive(state.pid) AND probe_socket(state.socket_path):
            → ADOPT: register in supervisor map, skip spawn
            → if resume_token exists: use it for ACP session reattach
        elif pid_is_alive(state.pid) AND NOT probe_socket:
            → STALE: SIGTERM → 5s → SIGKILL → clean state file → spawn fresh
        else:
            → DEAD: clean state file → spawn fresh
    else:
        → NEW: spawn fresh (current behavior)
```

For Zellij-hosted agents, the adoption probe is: `systemctl --user is-active <service>` (already exists) AND `zellij list-sessions` contains the session name (already exists). The state file adds process-level confirmation.

#### 1.4 Graceful shutdown

Register a SIGTERM handler in `daemon.rs` that:

1. Sets a `shutting_down` flag (prevents new spawns).
2. For each running agent:
   - ACP agents: send `session/end` via the ACP protocol.
   - Task-mode agents: send SIGTERM to the process.
   - Zellij-hosted agents: no action (they're independent).
3. Wait up to 10s for all agents to acknowledge or exit.
4. Write final state files (updated `resume_token` if available).
5. Exit.

If edgeplaned is SIGKILLed (no handler runs), agents survive because they're in their own session (1.1). On next startup, adoption (1.3) finds them alive.

#### 1.5 Unified runtime state for all runtimes

All runtime kinds get the same state file and adoption flow:

| Runtime | State file contents | Adoption probe |
|---------|-------------------|----------------|
| `claude_agent_acp` | pid, socket_path, resume_token | pid alive + socket responsive |
| `claude_code` (task) | pid, task_id | pid alive |
| `zellij_hosted` | systemd_service, zellij_session | systemctl is-active + zellij list-sessions |
| `codex` / `gemini` / `goose` | pid | pid alive |

---

## Phase 2: Graduated Isolation

### Goal

Agents get configurable isolation boundaries. The sandbox crate is wired into the spawn path. An adversarial validation agent proves the boundaries work.

### Isolation levels

#### `none`

Current behavior. Same user, same fs, same network. No namespace setup. For admin-owned agents that need full host access.

#### `namespace`

Lightweight isolation via Linux user namespaces + mount namespace:

- `unshare(CLONE_NEWUSER | CLONE_NEWNS)` — own user and mount namespace.
- No PID, network, IPC, or UTS namespace (too restrictive for most agent work — agents need host network for Tailscale services, LiteLLM, etc.).
- Filesystem view driven by `FsPolicy` from the agent spec:
  - **Default template:** own `state_dir` (RW) + system binaries and libraries (RO) + aria binary (RO) + claude binary (RO).
  - **Per-agent overrides:** `extra_ro_bind` and `extra_rw_bind` in the agent spec. Example: engineer profile gets `/home/merlin/code/edgeplane` as RO bind.
  - **Shared tmp:** configurable via `share_host_tmp` (default: false, isolated tmpfs).
- No seccomp, no capability drop. Trusted users who shouldn't accidentally clobber each other, not adversarial containment.
- `PR_SET_NO_NEW_PRIVS` is set (defense-in-depth, no operational cost).
- **Limitation:** No PID or network namespace at this level. Agents sharing a node can still signal each other's processes and read each other's `/proc` entries (same real UID). Process isolation requires `jailed` level. This is acceptable for trusted users — the fs boundary prevents accidental clobber, which is the primary goal.

#### `jailed`

Full sandbox. Everything in `namespace` plus:

- `pivot_root` into a synthetic tmpfs rootfs.
- Seccomp filter via `edgeplaned-sandbox/src/seccomp.rs` (baseline deny list + per-agent `extra_deny_syscalls`).
- Full capability drop via `prctl(PR_CAPBSET_DROP)` + `capset(2)`.
- Network namespace with egress allowlist (`NetworkPolicy.egress_allowlist`). Empty = all network denied.
- cgroup v2 limits (`CgroupLimits`): memory (default 512 MiB), max PIDs (default 64), CPU weight (default 100). All configurable per-agent.
- Binary hash verification at spawn (`resolve_and_hash_binary` + `verify_binary_hash`).
- Landlock execute restrictions.

### Spawn path integration

In `daemon.rs:spawn_one()`, after Phase 1's `setsid` setup:

```
let effective_isolation = clamp_isolation(
    agent_spec.isolation_level,
    agent_owner_role,  // Phase 3; default admin until then
);

match effective_isolation {
    None => { /* spawn as-is, setsid only */ }
    Namespace => {
        let fs_policy = resolve_fs_policy(&agent_spec);
        // child pre_exec hook: setup_namespace(fs_policy)
    }
    Jailed => {
        let jail_config = build_jail_config(&agent_spec);
        // child calls enter_jail(jail_config) on startup
    }
}
```

The `enter_jail()` function already handles the full lifecycle (fork, unshare, uid_map, mount setup, pivot_root, landlock, seccomp). The new work is:
- `build_jail_config()`: constructs `JailConfig` from agent spec's `FsPolicy`, `NetworkPolicy`, `CgroupLimits`, and declared capabilities.
- `setup_namespace()`: a lighter variant that does `unshare(CLONE_NEWUSER | CLONE_NEWNS)` + mount setup without pivot_root, seccomp, or capability drop.
- `resolve_fs_policy()`: merges the agent's declared bind mounts with the default template.

### Agent spec schema additions

```toml
# In agent manifest or profiles.toml
[[profile]]
name = "work"
isolation_level = "namespace"  # none | namespace | jailed

[profile.fs_policy]
extra_ro_bind = ["/home/merlin/code/aria"]
extra_rw_bind = []
share_host_tmp = false

[profile.network_policy]
egress_allowlist = []  # empty = unrestricted (namespace level ignores this)

[profile.cgroup_limits]
memory_mib = 1024
max_pids = 128
cpu_weight = 100
```

### Default templates

To avoid requiring every agent spec to declare full fs_policy, three built-in templates:

| Template | RO binds | RW binds | Network |
|----------|----------|----------|---------|
| `default` | `/usr/bin`, `/usr/lib`, `/lib`, aria binary, claude binary, `/etc/ssl` | `<state_dir>` | host (no namespace) |
| `developer` | `default` + `/home/<user>/code` (entire code dir) | `<state_dir>` + `<work_dir>` | host |
| `restricted` | Pinned binary + ldd deps only | `<state_dir>/tmp` only | egress allowlist only |

`none` uses no template. `namespace` defaults to `default`. `jailed` defaults to `restricted`. Per-agent overrides merge on top.

### Adversarial validation agent

A test binary (`edgeplaned-adversary`) and integration test suite that validates isolation boundaries. Runs at each isolation level and attempts:

| Test | `none` expected | `namespace` expected | `jailed` expected |
|------|----------------|---------------------|-------------------|
| Read `/home/<user>/.ssh/id_ed25519` | PASS | FAIL (ENOENT) | FAIL (ENOENT) |
| Write to another agent's `state_dir` | PASS | FAIL (ENOENT or EROFS) | FAIL (ENOENT) |
| `kill -9 <other_agent_pid>` | PASS | PASS (same real UID) | FAIL (EPERM, PID ns) |
| Read `/proc/<other_pid>/environ` | PASS | PASS (same real UID) | FAIL (EPERM, PID ns + procfs isolated) |
| Exec `/usr/bin/curl` (not in allowlist) | PASS | PASS | FAIL (EACCES) |
| Connect to `10.0.0.1:22` (not in egress) | PASS | PASS | FAIL (ENETUNREACH) |
| Allocate 1 GiB RAM | PASS | PASS | FAIL (OOM killed) |
| Fork 200 processes | PASS | PASS | FAIL (EAGAIN) |

This runs as:
- A CI job in the edgeplane repo (requires Linux runner with unprivileged user namespaces enabled).
- An on-demand `/eval` target via `edgeplane system adversary run --level <level>`.

---

## Phase 3: Role-Based Permission Model

### Goal

User identity determines the isolation ceiling. Admins can opt out of isolation. Members cannot. Guests are always jailed.

### Role definitions

| Role | Isolation ceiling | Can deploy with | Default isolation |
|------|------------------|----------------|-------------------|
| `admin` | `none` | `none`, `namespace`, `jailed` | `namespace` |
| `member` | `namespace` | `namespace`, `jailed` | `namespace` |
| `guest` | `jailed` | `jailed` only | `jailed` |

### Enforcement

**Enforcement point:** edgeplaned at spawn time, not tower.

Tower passes the agent owner's role in the agent assignment (via the existing `AgentSpec` or `AgentLaunchContext`). edgeplaned clamps:

```rust
fn clamp_isolation(requested: IsolationLevel, role: Role) -> IsolationLevel {
    let ceiling = match role {
        Role::Admin => IsolationLevel::None,
        Role::Member => IsolationLevel::Namespace,
        Role::Guest => IsolationLevel::Jailed,
    };
    requested.max(ceiling)  // stricter wins
}
```

If clamped, edgeplaned logs: `"isolation_level clamped: {requested} -> {effective} (role={role}, agent={agent_id})"`.

### Role assignment

| Auth method | Role source |
|-------------|-------------|
| OIDC (interactive login) | Authentik group claim `edgeplane_role` (`admin`, `member`, `guest`). Tower reads during session creation, stores on `usersession`. |
| Service account | Set at creation time via `POST /auth/service-accounts` (new `role` field, default `member`). |
| Node JWT | Inherits role of the user who created the join token. Stored in JWT claims. |

### Schema changes

**tower (Postgres):**
- `usersession`: add `role TEXT NOT NULL DEFAULT 'member'`
- `serviceaccount`: add `role TEXT NOT NULL DEFAULT 'member'`
- Node JWT claims: add `role` field

**edgeplaned (local registry):**
- `agent_launch_context`: add `owner_role TEXT NOT NULL DEFAULT 'admin'` (default admin for backwards compat with existing fleet profiles)

**Agent spec:**
- Add `isolation_level` field (`none` | `namespace` | `jailed`, default per role)
- Add `fs_policy`, `network_policy`, `cgroup_limits` (all optional, defaults from template)

### Your fleet today

All six profiles run via node JWT authenticated as your user (admin). Default isolation = `namespace`. Each profile can opt to `none` in `profiles.toml` if needed. Exact same behavior as today unless you change config.

---

## Migration & Backwards Compatibility

- **Phase 1:** No config changes required. Agents that were `kill_on_drop` become `setsid`. State files are new (additive). Adoption is additive (falls through to spawn-fresh on missing state file). Graceful shutdown is additive.
- **Phase 2:** `isolation_level` defaults to `none` until Phase 3 wires role enforcement. Existing fleet keeps current behavior. Operators opt into `namespace` or `jailed` per-agent.
- **Phase 3:** Role defaults to `admin` for existing fleet profiles (backwards compat). New OIDC users default to `member`. `isolation_level` default changes from `none` to per-role default.

## Testing Strategy

| Layer | How |
|-------|-----|
| Unit tests | `setsid` spawn, state file write/read, adoption logic, `clamp_isolation`, `build_jail_config` |
| Integration tests | Adversarial agent at each isolation level (see Phase 2 table) |
| CI | Adversarial suite on Linux runner. State file adoption round-trip. Graceful shutdown drain. |
| Live validation | Deploy on excalibur, restart edgeplaned, confirm agents survive. Deploy a `namespace` agent, confirm fs boundary. |

## Open Questions

None — all design decisions resolved during brainstorming.

## References

- `crates/edgeplaned/crates/edgeplaned-sandbox/` — existing jail implementation
- `crates/edgeplaned/crates/edgeplaned-acp/src/agent.rs:158` — current `kill_on_drop` spawn
- `crates/edgeplaned/crates/edgeplaned-bin/src/daemon.rs:spawn_one()` — spawn orchestration
- `crates/edgeplaned/crates/edgeplaned-bin/src/unit_health.rs` — systemd supervision loop
- Incus process adoption model — architectural inspiration

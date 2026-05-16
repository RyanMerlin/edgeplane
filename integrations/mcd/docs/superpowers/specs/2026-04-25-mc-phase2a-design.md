# mc Phase 2a Design: Unified CLI Surface + Library Crates

**Date:** 2026-04-25
**Status:** Approved
**Scope:** Phase 2a of the clix → mc-mesh consolidation. Phase 2b (TUI) is a separate spec.

---

## Mission

Make `mc` the single CLI surface for both human operators and AI agents interacting with the
capability gateway. `mc-mesh` becomes a pure daemon — nobody calls it directly. Agents and
humans both use `mc run`, `mc capabilities`, and `mc receipts`. `mc-mesh` enforces policy,
injects secrets, sandboxes execution, and writes receipts.

---

## Layer Map

```
┌─────────────────────────────────────────────────────┐
│  mc  (unified CLI — humans + agents)                │
│                                                     │
│  humans: mc tui / mc packs / mc receipts / mc init  │
│  agents: mc run <cap> --json / mc capabilities      │
│          mc receipts last                           │
└──────────┬──────────────┬──────────┬────────────────┘
           │ local socket │ remote   │ fallback
           │ (on-node)    │ TCP/TLS  │ (REST proxy)
           ▼              ▼          ▼
     Unix socket     TCP :7731   MC backend
     mc-mesh-        (Tailscale  REST API
     mgmt.sock       reachable)
           │              │
           └──────┬───────┘
                  ▼
        ┌──────────────────────┐   ┌──────────────────────────┐
        │  mc-mesh daemon      │   │  ~/.missioncontrol/      │
        │                      │   │  ├── receipts.db          │
        │  JSON-RPC listener   │──▶│  ├── sync/               │
        │  policy enforcement  │   │  └── mc-mesh-mgmt.sock   │
        │  secrets injection   │   └──────────────────────────┘
        │  sandbox exec        │
        │  receipt write       │
        └──────────────────────┘
```

**Design principle:** `mc` = kubectl. `mc-mesh` = kubelet. Agents never call `mc-mesh` directly.

---

## Routing Model

Capability commands (`mc run`, `mc capabilities`, `mc capabilities describe`) support four
routing modes:

| Mode | Behavior |
|------|----------|
| `auto` (default) | Local socket → remote TCP (if host registered with MC) → backend proxy |
| `local` | Unix socket on this machine; error if daemon not running |
| `remote` | Direct TCP to named `mc-mesh` host (Tailscale-reachable); `mc --host <node>` |
| `backend` | Route through MissionControl REST API |

**Local route** gives the daemon's full policy view of this node — used by on-node agents and
operators working directly on a node.

**Remote route** connects directly to `mc-mesh` on another node over Tailscale TCP port `7731`.
`mc` resolves the node's address from the MC backend (registered at daemon startup), then speaks
the same JSON-RPC protocol over an authenticated TCP connection. This is the primary route for
operators managing a remote node — faster and more direct than the backend proxy.

**Backend route** routes through the MissionControl REST API. Useful when the target node is
offline or not Tailscale-reachable, or when the operator wants the control plane's view of
mission-scoped capabilities rather than the node's local policy.

**`auto` resolution order:**
1. If `MC_MESH_SOCKET` is set and the socket file exists → `local`
2. Else if MC backend reports a reachable address for this host → `remote`
3. Else → `backend`

### Remote TCP authentication

`mc-mesh` binds TCP port `7731` (configurable via `MC_MESH_MGMT_PORT`) in addition to the Unix
socket. Each TCP connection must present the node's bearer token (same `MC_TOKEN` used for
backend auth) in a one-line handshake before any JSON-RPC exchange:

```
Client → Server:  AUTH <token>\n
Server → Client:  OK\n   (or ERR <reason>\n)
```

After `OK`, the connection proceeds with normal newline-delimited JSON-RPC. The Unix socket
requires no auth (same-user Unix permissions enforce access). The TCP port binds on all
interfaces by default so Tailscale can reach it; a firewall rule or Tailscale ACL restricts
access to the operator's devices.

### `mc --host` flag

```
mc --host optiplex capabilities
mc --host optiplex run kubectl-observe.kubectl-get-pods --json
```

`--host` sets `remote` mode with an explicit address. Value: `<hostname>` (resolved from MC
backend node registry) or `<hostname>:<port>` (direct override, e.g. `optiplex:7731`). Stored
in `~/.missioncontrol/config.json` as `default_host` for session-level default.

Configuration priority (highest to lowest):
1. `mc --host <node>` CLI flag → implies `remote` mode
2. `mc --route <mode>` CLI flag
3. `MC_ROUTE` / `MC_MESH_HOST` environment variables
4. `capability_route` / `default_host` fields in `~/.missioncontrol/config.json`
5. Default: `auto`

Agents spawned by `mc-mesh` always get `local` in practice — `MC_MESH_SOCKET` is set in their
env and the daemon is always running when they are.

---

## New Library Crates

### `mc-mesh-receipts`

SQLite audit store at `~/.missioncontrol/receipts.db`. Written by the daemon on every `mc run`
execution. Read directly by `mc receipts` — no daemon needed for browsing.

**Schema:**

```sql
CREATE TABLE receipts (
    id               TEXT PRIMARY KEY,   -- uuid4
    capability       TEXT NOT NULL,      -- "kubectl-observe.kubectl-get-pods"
    args_json        TEXT NOT NULL,      -- caller-supplied args
    result_json      TEXT NOT NULL,      -- full DispatchResult JSON
    exit_code        INTEGER NOT NULL,
    execution_time_ms INTEGER NOT NULL,
    mission_id       TEXT,               -- nullable
    agent_id         TEXT,               -- nullable
    created_at       TEXT NOT NULL       -- ISO8601
);
CREATE INDEX receipts_created_at ON receipts(created_at DESC);
CREATE INDEX receipts_mission_id ON receipts(mission_id);
```

**Public API:**

```rust
pub struct ReceiptStore { /* path to receipts.db */ }

impl ReceiptStore {
    pub fn open(path: &Path) -> Result<Self>;
    pub fn insert(&self, receipt: &Receipt) -> Result<()>;
    pub fn last(&self, limit: usize) -> Result<Vec<Receipt>>;
    pub fn get(&self, id: &str) -> Result<Option<Receipt>>;
    pub fn list(&self, filter: ReceiptFilter) -> Result<Vec<Receipt>>;
}

pub struct ReceiptFilter {
    pub mission_id: Option<String>,
    pub agent_id: Option<String>,
    pub capability: Option<String>,
    pub limit: usize,
    pub since: Option<DateTime<Utc>>,
}
```

Uses SQLite WAL mode for safe concurrent reads from `mc` while daemon writes.

---

### `mc-mesh-sync`

Git-backed bidirectional sync of curated configuration. Local cache at
`~/.missioncontrol/sync/`.

**Repo layout (expected by the crate):**

```
fleet/
  packs/          ← protected; PR required to change
  policy/         ← protected; PR required to change
  harnesses/      ← agent harness templates (goose/, claude/, codex/, gemini/)
nodes/
  <hostname>/
    packs/        ← node-owned; free push from this node
    policy/       ← node-owned; free push from this node
    overrides/    ← node-specific config overrides
```

Fleet-wide config requires a pull request to `main`. Node-specific config lives in
`nodes/<hostname>/` — this node pushes to its own branch freely. Agents (via goose) and
human operators (via TUI) can both propose node-scoped changes.

**Public API:**

```rust
pub struct SyncClient { /* repo_url, local_cache_dir, hostname */ }

impl SyncClient {
    pub fn new(repo_url: &str, cache_dir: &Path, hostname: &str) -> Result<Self>;
    pub fn pull(&self) -> Result<SyncResult>;
    pub fn push_node_changes(&self, message: &str) -> Result<PushResult>;
    pub fn status(&self) -> Result<SyncStatus>;
    pub fn last_pulled_at(&self) -> Result<Option<DateTime<Utc>>>;
}

pub struct SyncStatus {
    pub last_pulled_at: Option<DateTime<Utc>>,
    pub last_pushed_at: Option<DateTime<Utc>>,
    pub node_branch_dirty: bool,     // uncommitted local changes in nodes/<hostname>/
    pub fleet_branch_ahead: u32,     // commits behind main in fleet/
}
```

`push_node_changes` commits all dirty files under `nodes/<hostname>/` and pushes to a
`nodes/<hostname>` remote branch. It does NOT touch `fleet/`. PR creation for fleet changes
is out of scope for this crate — surfaced in the TUI (Phase 2b).

---

## Management Gateway

`mc-mesh` daemon exposes the management interface on two transports simultaneously:

### Unix socket (local access)

Path: `~/.missioncontrol/mc-mesh-mgmt.sock`, permissions `0600`.
`MC_MESH_SOCKET` env var (injected by Phase 1 runtimes) points here.
No auth — Unix file permissions enforce same-user access.

### TCP listener (remote access)

Binds `0.0.0.0:7731` (configurable: `MC_MESH_MGMT_PORT` env or `mgmt_port` in config).
Reachable over Tailscale from the operator's machine.

One-line auth handshake before any RPC:
```
Client → Server:  AUTH <MC_TOKEN>\n
Server → Client:  OK\n   (or ERR unauthorized\n → connection closed)
```

After `OK`, identical newline-delimited JSON-RPC 2.0 protocol as the Unix socket.

### Protocol (both transports)

```jsonc
// Dispatch a capability
{"jsonrpc":"2.0","id":1,"method":"dispatch","params":{
  "full_name": "kubectl-observe.kubectl-get-pods",
  "args": {"namespace": "default"},
  "dry_run": false,
  "timeout_secs": 30,
  "mission_id": "m1",
  "agent_id": "a1"
}}

// List capabilities (policy-scoped)
{"jsonrpc":"2.0","id":2,"method":"capabilities.list","params":{"tag":"kubernetes"}}

// Describe a capability (schema + examples)
{"jsonrpc":"2.0","id":3,"method":"capabilities.describe","params":{
  "full_name": "kubectl-observe.kubectl-get-pods"
}}
```

### Node address registration

At daemon startup, `mc-mesh` registers its Tailscale/external address with the MC backend
(new field on the node record: `mgmt_addr: "optiplex:7731"`). `mc` resolves this address when
`--host optiplex` is used and no explicit port is given.

---

## `mc` CLI Extensions

All new subcommands land in the existing `mc` binary (`integrations/mc/`).

### Capability commands

```
mc capabilities [--tag <domain>] [--json] [--route auto|local|backend]
mc capabilities describe <pack>.<capability>
mc run <pack>.<capability> [--json] [--dry-run] [--timeout <secs>] [--route ...]
```

`mc capabilities` (listing) always asks the daemon (policy-scoped list). Falls back to the
embedded `PackRegistry` with a warning if daemon is unreachable:
`⚠ daemon offline — showing all built-ins, policy not applied`
`--route` applies to listing only.

`mc capabilities describe` reads from the embedded `PackRegistry` — schema doesn't vary
with policy, offline-safe, no `--route` flag (always local).

`mc run` routes to daemon (local) or MissionControl backend (remote) per routing config.

**Output contract** (identical to Phase 1 dispatcher):
- `--json` or non-TTY stdout: `{"ok":true,"data":{...},"receipt_id":"..."}` / `{"ok":false,"error":"...","hint":"...","example":"mc run ..."}`
- TTY without `--json`: human-readable pretty output with color

### Receipt commands

```
mc receipts last [--json] [--limit N]
mc receipts get <id> [--json]
mc receipts ls [--limit N] [--mission <id>] [--agent <id>] [--json]
```

All read directly from `~/.missioncontrol/receipts.db` — no daemon needed.

### Sync command

```
mc sync [--force]
mc sync status
mc sync push [--message "..."]
```

`mc sync` pulls from the configured git repo. `mc sync push` commits and pushes dirty files
under `nodes/<hostname>/` to the node's branch. `mc sync status` shows last pull/push times
and dirty state.

Sync repo URL configured via `sync_repo` in `~/.missioncontrol/config.json` or
`MC_SYNC_REPO` env var.

### `mc init` extensions

```
mc init [--from-repo <git-url>] [--profile <name>]
```

Extends the existing `mc init`. When `--from-repo` is supplied, performs full node bootstrap:

1. Clone sync repo → `~/.missioncontrol/sync/`
2. Store `INFISICAL_TOKEN` (from env) → OS keyring
3. Apply AppArmor profile from `sync/fleet/apparmor/` (or bundled fallback)
4. Render goose harness config from `sync/fleet/harnesses/goose/` → well-known paths
5. Register node with MissionControl backend
6. Install + enable systemd unit for `mc-mesh` daemon

Only goose's harness is rendered at init. Other agents (claude, codex, gemini) are rendered
on-demand by `ensure_installed()` when a mission requires them.

One-curl bootstrap:
```bash
INFISICAL_TOKEN=<token> curl -fsSL https://<release>/mc/install.sh | sh -s -- \
  --profile agent-node \
  --repo https://git.example.com/mc-config
```

---

## Agent System Prompt Footprint

After Phase 2a, the harness template rendered into an agent's config contains:

```markdown
## Capabilities
Discover: `mc capabilities [--tag <domain>]`
Detail:   `mc capabilities describe <pack>.<capability>`
Execute:  `mc run <pack>.<capability> --json [--dry-run]`
History:  `mc receipts last [--json]`
```

Four lines. No MCP tool dump. No capability schema preloaded. Progressive discovery on demand.

---

## Implementation Phases

### Phase 2a-1: Library crates
- `mc-mesh-receipts` — SQLite store, WAL mode, ReceiptStore API, tests
- `mc-mesh-sync` — git pull/push, node-scoped partition, SyncStatus, tests
- Add both to mc-mesh workspace `Cargo.toml`
- Wire `mc-mesh-receipts` into capability dispatcher (`capability_dispatcher.rs`) — insert
  receipt after every `dispatch()` call

### Phase 2a-2: Management gateway in mc-mesh daemon
- Add `mc-mesh-mgmt.sock` Unix listener to `crates/mc-mesh/src/mgmt_gateway.rs`
- Add TCP listener on `0.0.0.0:7731` with one-line AUTH handshake before JSON-RPC
- JSON-RPC 2.0 handler for `dispatch`, `capabilities.list`, `capabilities.describe`
- Unix socket `0600` permissions, path exported as `MC_MESH_SOCKET`
- Register `mgmt_addr` (hostname:port) with MC backend at daemon startup
- Existing attach socket (`mc-mesh.sock`) unchanged

### Phase 2a-3: `mc` CLI extensions
- Add `mc-mesh-packs`, `mc-mesh-receipts`, `mc-mesh-secrets` as dependencies to `integrations/mc/Cargo.toml`
- New `McDispatch` routing layer (socket + backend fallback)
- Add subcommands: `mc run`, `mc capabilities`, `mc receipts`, `mc sync`
- Extend `mc init` with `--from-repo` bootstrap flow
- Wire `MC_MESH_SOCKET` and routing config into `mc`'s config resolution

---

## Out of Scope (Phase 2b / later)

- TUI (receipts browser, packs browser, mission console) — Phase 2b
- Fleet-wide PR workflow for `fleet/` changes — Phase 2b TUI
- `mc-mesh-receipts` sync to MissionControl backend — deferred until northbound API finalized
- Remote `mc --host <node>` direct socket tunnel — deferred; backend route covers the use case

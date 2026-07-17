# Changelog

All notable changes to edgeplane, edgeplaned, and edgeplane-tower are recorded here. The three binaries ship in lockstep — `/VERSION` is the source of truth and `scripts/set-version.sh <new>` bumps all three in one step.

This project follows semantic versioning where possible, but pre-1.0 minor bumps may include breaking changes when the cost of a major bump outweighs the signal value.

## [0.16.1] — 2026-07-17

### Added

- **`edgeplane task mesh <verb>` (#111).** CLI mirror of all 9 `mcp__edgeplane__*_mesh_task`
  tools (`submit`/`claim`/`get`/`list`/`heartbeat`/`progress`/`complete`/`fail`/`block`),
  closing the gap where the real, agent-claimable `meshtask` model was reachable only via
  MCP or raw REST — EdgePlane's design goal is CLI-first, MCP only where no CLI equivalent
  exists. Kept as a distinct `mesh` subcommand under `task` rather than folding into it,
  since legacy `task create/list/show/update/delete` operates on the completely
  disconnected, UI-only `task` table (see 0.16.0's #108 entry above).

### Fixed

- **`progress_mesh_task` MCP tool 500'd on every call (#111).** Its `meshprogressevent`
  INSERT never set `seq` (NOT NULL, no DB default) — same missing-column class as #110.
- **Progress-event sequence numbers have always been stuck at 0 (#111).** Both the new MCP
  handler and the REST `post_progress` handler (the one `edgeplaned-work`'s daemon client
  actually calls) decoded the `SELECT COALESCE(MAX(seq),-1)+1` query as `i64`, but `seq` is
  Postgres `integer` (i32) — sqlx's runtime decode silently failed every time and fell
  through to `unwrap_or(0)`. Every progress event ever posted, on either path, got `seq=0`
  regardless of how many prior events existed for the task. Fixed the type in both files.

## [0.16.0] — 2026-07-17

OSS hardening release: security hardening across auth, nodes, and the daemon install story;
breaking identity rebrands completing the MissionControl → EdgePlane migration. Also folds in
a full authorization-hardening audit (closing pre-existing cross-domain IDORs and moving node
credentials from full-trust to domain-scoped), Phase 1a of first-class secrets, and a
live-tower task-claim regression fix.

### Security

- **Node-JWT TTL reduced 90 days → 24 hours (#72).** `NODE_JWT_TTL_DAYS` env var (tower,
  default 1) controls the window.  `edgeplaned` auto-rotates its node JWT every 12 hours via
  a background task — the live token is held in `Arc<RwLock<String>>` and persisted atomically
  at `0600` so both WS reconnect paths always read the current credential without a restart.
- **Node self-auth on reconcile endpoints (#71).** A node's tower-signed RS256 JWT
  (`auth_type == "node"`, `sub == "node:{node_id}"`) is now authorized on its own
  `list_node_agents` and `node_notify_ws` endpoints.  Cross-node forgery, path-traversal,
  and replay attacks are blocked; handler fails closed on DB error.  Flips headless-federation
  403 → 200.
- **OIDC `email_verified` gate (#77, #79).** Browser OIDC logins require `email_verified: true`
  from the IdP.  Device-flow (`edgeplane auth login`) gains equivalent enforcement at the
  exchange step.
- **`quinn-proto` 0.11.14 → 0.11.15 (#74).** Resolves RUSTSEC-2026-0185.
- **Authorization hardening — Workstream 1: closed broken-access-control gaps across the
  tower (#96–#104).** A tower authorization-surface audit found unauthenticated or ungated
  cross-domain access across REST and MCP. Fixed by domain: 13 cross-domain read IDORs
  (Group A, #97), dead agent-handler deletion + gated write mutations (Group B, #99, #100),
  feedback/ingestion cross-tenant reads (Groups E/G, #98), NULL-domain fail-open artifact/doc
  writes + `create_job`/`record_usage_batch` domain gate (Groups C/E-remainder, #103),
  `remotectl create_launch` reject node/agent + clamp ttl/scope (Group D, #102), and search
  readability narrowed from substring-LIKE to exact membership (Group F, #104). Also closes a
  live IDOR unmasked by the flat mission-brief `archived_at` correctness fix (#96) — the
  endpoint was a dead 500 for every caller until the bad predicate was removed, which would
  otherwise have exposed an ungated cross-domain read+write. 25 read-path + 7 write-path cases
  red-teamed live against prod with real domain-scoped tokens.
- **Authorization hardening — Workstream 2: node credentials are domain-scoped, not
  full-trust (#106, #107).** Node JWTs previously bypassed domain checks entirely
  (`auth_type == "node"` was a blanket grant in `authorized_for`). Node scope is now dynamic,
  derived from the node's assigned `meshagent` rows, and invalidated on assign/revoke/detach/
  enroll/delete. A follow-up (#107) closed a gap an adversarial review found in the shipped
  fix: `assign_node_agent` itself wasn't authorizing the caller against the target domain
  before seeding the `meshagent` row, which could reopen the same hole it fixed. 6/6 live
  red-team on each.
- **Join-token privilege escalation closed (#90, #92).** Any authenticated principal —
  including node and agent JWTs — could mint or rotate node-enrollment join tokens.
  `create_join_token` and `rotate_join_token` now reject `node`/`agent`-type credentials;
  only session/service-account principals can mint tokens, scoped to their own identity. The
  web `/admin` route also gained a `beforeLoad` redirect guard — the previous nav-hiding was
  cosmetic and didn't stop direct URL navigation.

### Breaking

- **Auth token prefix: `mcs_` → `ep_`.** Session, service-account, and client-secret tokens
  now use `ep_` / `ep_sa_` / `ep_cs_` prefixes.  Clean cutover — existing tokens are invalid
  and must be re-issued (`edgeplane auth login`).  No DB migration.
- **`edgeplaned` env vars: `MCD_*` → `EP_*`.** `MCD_WORK_DIR` → `EP_WORK_DIR`,
  `MCD_CRON_FILE` → `EP_CRON_FILE`.  The old names are no longer read.
- **Goose runtime removed.** `GooseRuntime`, `dispatch = "goose"` cron mode, the
  Phase-3 task-triage loop, and `EP_GOOSE_BIN` / `MCD_GOOSE_BIN` are gone.  Default
  home-domain runtime is now `claude_code`.  **Upgrade:** remove `dispatch = "goose"` jobs
  from `cron.toml` before upgrading — a leftover goose job fails whole-config validation and
  silently disables all cron jobs.
- **`edgeplaned` as the canonical system service (#70).** `edgeplane node run` is a
  one-release deprecation stub.  Switch to the `edgeplaned` systemd unit and
  `edgeplane agent node register` enrollment flow.

### Added

- **Group-based admin via OIDC `groups` claim (#80).** `EP_ADMIN_GROUPS` (comma-separated)
  grants admin to users whose IdP groups intersect the list.  Persisted on the session at
  login (migration 0011); re-evaluated on each request.
- **`is_admin` in `edgeplane auth whoami` (#78).** Whoami JSON and CLI summary now include
  `is_admin: true/false`.
- **365-day configurable login TTL + CLI admin tokens (#76).** `EP_SESSION_TTL_DAYS`
  controls browser-session lifetime (default 365).  `edgeplane auth admin-token` mints
  long-lived admin service tokens without a browser flow.
- **Node-delete (#73).** `DELETE /api/runtime/nodes/{id}?force` removes a node (owner or
  admin only; node-self rejected).  Returns 409 if assigned agents exist unless `?force`
  detaches them first.  Revokes node JWT + join token in a single transaction.
  CLI: `edgeplane agent node delete <id> [--force]`; dashboard: DeleteNodeButton with
  force-confirm.  `edgeplane agent node ls` lists nodes visible to the current principal.
- **Nightly restart is now configurable.** `EP_NIGHTLY_RESTART_HOUR` (0–23, default 3)
  and `EP_NIGHTLY_RESTART_ENABLED` (true/false) control the daemon maintenance window.
- **First-class secrets, Phase 1a (#93).** `SecretsBackend` trait (async, dyn-safe) with
  `Env` and `Infisical` implementations behind a scheme-routed `BackendRegistry`;
  `CredentialKind::Ref` is the canonical secret-reference form with legacy back-compat
  preserved. Also fixes a bug where the Infisical resolution path was silently inert (the
  daemon built the capability dispatcher with no registry, so `Ref`/`Infisical` credentials
  fell through). Phase 1b (JIT broker) is not yet built.
- **System-mode node enrollment script (#91).** `scripts/install-edgeplane-node.sh`
  downloads `edgeplaned` from GitHub releases, verifies its SHA256 checksum, creates the
  dedicated `edgeplane` system user, installs the hardened systemd unit, and enrolls the
  node — replacing the old binary-required `install.sh` path.

### Hardening

- **XDG-aware path resolution (#69).** `edgeplaned-paths` is the single home-directory
  SSOT: honors `EP_HOME` > `XDG_CONFIG_HOME` / `XDG_STATE_HOME` / `XDG_DATA_HOME` >
  `~/.edgeplane/{bucket}`.  Duplicated logic in `edgeplaned-sync` and `edgeplane-tower`
  (which fell back to `/root/.edgeplane`) is collapsed into this crate.
- **Non-root `edgeplaned` system service (#70).** Production install runs as a dedicated
  `edgeplane` system user with a hardened unit (`ProtectSystem=strict`, `ProtectHome=yes`,
  `NoNewPrivileges=yes`, `PrivateTmp=yes`, `RestrictNamespaces=user`, `StateDirectory=edgeplane`).
  Node credentials are `edgeplane:edgeplane 0600` and readable by the daemon.

### Fixed

- **Tower Dockerfile used a non-existent `edgeplaned-paths` source path (#75).**
- **`edgeplane auth whoami` / `Logged in as` showed the IdP subject instead of the display
  name (#81).**
- **Tower Helm chart's alembic initContainer was dead on arrival (#88).** The tower
  auto-migrates via `sqlx` on startup; the Python/alembic initContainer had no Python in the
  Rust image and never worked. Removed; chart bumped to 0.2.0.
- **RUSTSEC advisories + stale lockfile (#94).** `crossbeam-epoch` and `anyhow` patched;
  two `quick-xml` DoS advisories in an upstream-pinned `rust-s3` transitive dependency
  documented and ignored (XML is parsed only from our own object-storage responses, not
  attacker-controlled). `Cargo.lock` also resynced — it had drifted to pin workspace crates
  at 0.15.1 against 0.16.0 manifests.
- **Claims-integrity pass: docs matched to what's actually built, plus three correctness
  fixes (#95).** README/PHILOSOPHY/site docs no longer claim unbuilt features (overlap
  detection, HMAC governance, pgvector semantic search, S3 artifact-content tier) as shipped
  — reframed as roadmap. `install.sh`'s `curl | bash` path is fixed (died on unbound
  `BASH_SOURCE`). Also: artifact download no longer panics on a malformed `mime_type` (was a
  user-triggerable 500 via `unwrap()`), `system compat` no longer reports fabricated
  pass/warn for unimplemented checks, and `publish_execute` returns 409 instead of silently
  diverging Postgres from the Git ledger of record when the git push fails.
- **`SessionMode::Task` claim/heartbeat/complete/progress/fail 404'd against the live tower
  (#108).** `edgeplaned-work`'s HTTP client was missing the `/work` path segment on 8
  request paths across 7 functions; `poll_ready_tasks` swallowed the resulting error via
  `unwrap_or_default()`, so the failure mode was an agent silently seeing zero ready tasks
  rather than a visible error. Also fixes `load_mission_workspace`'s MCP snapshot, which was
  querying the legacy, UI-only `task` table instead of `meshtask` — the table every real
  claim/heartbeat/complete actually reads and writes — so the snapshot never reflected real
  agent work.

## [0.15.1] — 2026-06-19

Red-team hardening of the v0.15.0 P0 security release — closes read-side cross-domain authz gaps and intra-domain IDORs (no cross-domain *mutation* bypass or token escalation existed; the model held). Ships the daemon fail-closed token fallback to the fleet.

### Security

- **Read-side cross-domain authz closed (#62).** Domain authorization now enforces on all
  read-side MCP arms: `list_mesh_messages` (was a HIGH system-wide message-body broadcast
  accessible to any valid token), `get_domain_northstar`, `resolve_publish_plan`,
  `get_overlap_suggestions`, `list_mesh_tasks`, and `get_mesh_task`.
- **Intra-domain owner and self-identity checks (#62).** `authz_task_owner`/self-identity
  enforcement added to `progress_mesh_task`, `append_progress`, `unblock_task`, `create_gate`,
  and agent self-mutation paths (`agent_heartbeat`, `set_agent_status`, `update_agent_profile`).
  A compromised agent is now bounded to its own tasks and its own identity within its domain.
- **`send_mesh_message` sender-spoof closed (#62).** Sender identity is now verified against
  the authenticated principal; a token cannot impersonate a different sender.
- **Daemon token fallback explicitly fail-closed (#62).** On per-agent token mint failure the
  daemon removes the env var rather than leaving the previous value; no silent open fallback.
- **Revoke-on-agent-delete (#62).** Deleting an agent immediately revokes its active JWT,
  closing the window between deletion and natural token expiry.

## [0.15.0] — 2026-06-19

P0 security release — the two seams of the layered-tenancy hardening.

### Security

- **Seam 1 — domain authorization.** Default-deny `authorized_for_domain` on every
  privileged dispatch/ledger/stream handler in edgeplane-tower (REST + MCP), plus
  per-task lease ownership enforcement on lifecycle mutations and admin-gating of
  the previously-unauthenticated `global_sse` stream. Closes the hole where any
  authenticated token could dispatch or mutate work in any domain. Also closed an
  artifact-exfil gap in `get_artifact_download_url`.
- **Seam 2 — per-agent identity.** Each enrolled agent now gets its own
  short-lived, domain-scoped per-agent JWT (`AgentClaims`, the `agenttoken`
  revocation table via migration `0010`, fail-closed auth extractor) instead of
  the shared `EP_AGENT_TOKEN`. Tokens are minted at enrollment and via a
  full-trust-gated `POST /work/agents/{agent_id}/token` endpoint — agents cannot
  mint peer tokens. `claim`/`progress` are attributed to the authenticated agent
  (REST + MCP), and the daemon injects each agent's own token as its
  `EP_AGENT_TOKEN`, falling back to the shared daemon token if minting is
  unavailable so a mint hiccup degrades gracefully rather than breaking the fleet.

### Changed

- The daemon only mints per-agent tokens for runtimes that consume them
  (`claude_code`, `goose`); other runtime kinds keep using the shared daemon token,
  avoiding wasted mint round-trips and 404s for node-runtime agents (#57).

## [0.14.1] — 2026-06-15

### Changed

- **`edgeplane update` now converges every installed edgeplane binary, not just the CLI.**
  The self-update manifest (`latest.json`) gains a `bin` discriminator and lists
  `edgeplaned` alongside `edgeplane`; `update` replaces the running CLI plus any
  sibling binary already installed in the same directory (e.g. `edgeplaned` on a
  node), skipping byte-identical files. Pre-0.14.1 CLIs stay compatible — they
  ignore the unknown `bin` field, and entries are emitted sorted so `edgeplane-*`
  still resolves first.

### Added

- **Node self-update tooling** — `scripts/edgeplane-self-update.sh` (release-cadence
  node updater) plus systemd `edgeplane-update.{service,timer}`, and a one-shot
  `scripts/converge-node-0.14.0.sh` drift-remediation script. The timer is provided
  but **not yet wired into the installer** pending a fleet-restart design decision
  (restarting `edgeplaned` bounces co-located agent sessions on the node).

- **Web UI: display name from OIDC `preferred_username` claim.**
  `edgeplane-tower` now captures `preferred_username` (falling back to `name`) from the
  OIDC userinfo response at browser login and stores it in a new `usersession.display_name`
  column (migration `0004`). `GET /api/auth/me` returns the value as `name`; the SPA
  auth store exposes `userName` and uses it for the sidebar avatar and label. Single-word
  names ("Merlin") render as a single initial ("M"); multi-part names or emails use
  first+last initials.

- **Web UI: flat account popup with left-aligned items.**
  The sidebar's account popover is now a flat list — Preferences, Onboarding, Theme,
  Logout — with no Settings submenu. All items are explicitly left-aligned via
  `justifyContent: flex-start` (overriding the global `button { justify-content: center }`
  rule in `app.css`).

### Fixed

- **attach-ws: prompt frames from chat-UI viewers now reach `zellij_hosted` agent PTYs.**
  The PTY pump in `attach_ws.rs` ignored `{"kind":"prompt","text":"..."}` text frames
  (treating them as unknown control frames) — messages typed in the web UI were silently
  dropped. The pump now converts prompt frames to `text + \n` bytes sent to PTY stdin, so
  the chat UI works for all `zellij_hosted` fleet agents.

## [0.13.1] — 2026-06-07

### Fixed

- **Tower image for v0.13.0 never published — `edgeplane-zrpc-proto` was missing from the Dockerfile.**
  #17 added `edgeplane-zrpc-proto` as a workspace member but not to
  `crates/edgeplane-tower/Dockerfile`, which COPYs + stubs each member individually, so
  `cargo build -p edgeplane-tower` failed to load the workspace ("failed to load manifest for
  workspace member edgeplane-zrpc-proto"). The v0.13.0 CLI/daemon binaries + GitHub Release
  published fine (built directly, not via the tower Dockerfile), but the
  `ghcr.io/ryanmerlin/edgeplane:0.13.0` image build failed. The missing COPY + stub entry was
  fixed on main in #21; this patch release rolls that fix into a tagged build so the tower image
  publishes. No functional changes vs 0.13.0.

### Added

- **Zellij control-path plugin (`edgeplane-zrpc`) + daemon integration — dormant, feature-flagged.**
  A WASM control plugin gives `ZellijHosted` agents focus-free inject/cancel, scrollback
  reads, pane manifest, and pane-lifecycle events over Zellij pipes — replacing the
  paste→300ms→Enter chain with a focus-race-free path. Gated by `EDGEPLANE_ZRPC_PLUGIN_PATH`
  + `EDGEPLANE_ZRPC_SESSIONS` (both unset = no behavior change), so it ships dormant.
  Notable details: the plugin is built as a **bin crate** so it exports `_start` (a
  `cdylib` on `wasm32-wasip1` is a WASI reactor with no `_start`, which zellij 0.44.3
  rejects at instantiation); idempotent install tooling writes the session's
  `plugins{}`/`load_plugins{}` config (via the KDL parser, not string surgery) and a
  pre-seeded `permissions.kdl` (raw-path key); a lifecycle event consumer reads the
  `zrpc-events` pipe; and the request path reads-until-response then reaps the
  `zellij pipe` child rather than hanging on it.

### Breaking changes

- **`edgeplane launch` removed — `edgeplane run <runtime>` is the single agent launcher.**
  All agents launch through `run`: `claude`, `codex`, `goose` (native, profile-scoped
  homes with `doctor`/`exec`/`status`) and `gemini`, `openclaw`, `custom` (driver agents
  with instance isolation). `edgeplane launch <agent>` now errors with an
  unrecognized-subcommand message.

### Fixed

- **Claude lifecycle hooks were silently dead on the `run` path.** The generated hook
  wrappers invoked `edgeplane claude hook <event>` (no such subcommand) and `run_hook`
  POSTed to `/hooks/claude/*` without the `/api` prefix. Both fixed — session
  registration, context injection, tool-audit, and session-end now work. Compaction
  re-injects context too (the SessionStart matcher now includes `compact`).
- claude: a failed `--resume` clears the stale session id before retrying fresh.
- codex: a resume + fresh double-failure reports both exit codes and a next step.
- solo supervisor: the heartbeat thread no longer panics if its runtime fails to build.

### Changed

- `gemini`/`openclaw`/`custom` are now first-class `run` runtimes (gemini was a shim;
  openclaw/custom were `launch`-only).
- Internal rebrand: `McConfig`→`EdgeplaneConfig`, `McCommand`→`EdgeplaneCommand`,
  `McDispatch`→`EdgeplaneDispatch`, `McSyncConfig`→`EdgeplaneSyncConfig`; residual
  "MC"/MissionControl user-visible strings rebranded.
- Removed dead `#[cfg(test)]` Claude launch scaffolding; centralized edgeplane-binary
  MCP-command resolution in `config::resolve_ep_command`.
- `run goose doctor|status` return a clear message instead of "unknown runtime";
  `--new`/`--with-rtk` emit a note for runtimes that ignore them.
- Removed the undocumented `nanoclaw` alias and the unimplemented `--daemon-timeout` /
  no-op `--no-daemon` flags from docs.

### Known gaps

- Profile-sync live notification (`sessions_for_profile`) still tracks only driver-agent
  instances; wiring native `run` sessions into that index is a tracked follow-up.

## [0.12.0] — 2026-05-29

### Breaking changes

- **CLI: `--node-name` renamed to `--hostname`** for node registration (`edgeplane` / `edgeplaned`).

### API

- **`/api` prefix migration.** All edgeplane-tower routes (health, hooks, auth, work) are now served under `/api`. CLI, integration tests, and health probes updated. Callers must target `<base>/api`; the OIDC callback is now `https://<host>/api/auth/oidc/callback`.

### Web / fleet dashboard

- Design-system rewrite; the fleet dashboard (tabbed terminal views) is now the homepage.
- Removed token-based web login; OIDC login redirect fixed via Authentik.
- Fixed broken pages and the avatar menu.

### edgeplaned

- PTY bridge for ZellijHosted agents.

### CI / tooling

- GitHub Actions pinned to node24 release SHAs; Dependabot auto-merge for patch/minor; `/api/*` test paths; `set-version.sh` targets the root workspace `Cargo.toml`; schema-pack path fix.

### Docs

- GA4 tracking; agent isolation & lifecycle architecture spec; docs deploy to Cloudflare Pages.

> Supersedes the undocumented **0.11.2** release (design-system rewrite, PTY bridge, `/api` prefix), which bumped the version without a changelog entry.

## [0.11.1] — 2026-05-26

### CI fixes

- **Action SHA pins fixed:** `actions/create-github-app-token` and
  `softprops/action-gh-release` had truncated commit hashes causing
  "unable to resolve action" failures on every push and tag
- **Security audit:** added `audit.toml` ignoring two transitive-dep
  advisories — `RUSTSEC-2026-0002` (lru via ratatui) and
  `RUSTSEC-2024-0442` (wasmtime-jit-debug) — both unsound warnings
  with no exercised code path
- **GitHub Pages:** enabled Pages on the repo (was never configured,
  causing deploy-docs 404s)

### Action version bumps

All GitHub Actions bumped to latest major — resolves Node.js 20
deprecation warnings (actions forced to Node.js 24 starting June 2nd):

- `actions/checkout` v4 → v5
- `actions/create-github-app-token` v2 → v3
- `actions/deploy-pages` v4 → v5
- `actions/download-artifact` v4 → v5
- `actions/setup-node` v4 → v5
- `actions/upload-artifact` v4 → v5
- `actions/upload-pages-artifact` v3 → v4
- `docker/login-action` v3 → v4
- `softprops/action-gh-release` v2 → v3

---

## [0.11.0] — 2026-05-25

### Breaking changes

- **`EP_TOKEN` removed.** The static shared-secret authentication path has been
  removed from edgeplane-tower. All callers must authenticate via OIDC
  (interactive), node JWT (machine-to-machine), or service account tokens
  (programmatic). Deployments that relied on `EP_TOKEN` must migrate to one of
  these mechanisms before upgrading. See the [auth documentation](https://edgeplane.ai/guides/oidc/).

### Node JWT authentication

- RS256 JWT module for node identity — sign, verify, and revoke per-node tokens
- `POST /runtime/nodes/register` — public registration endpoint, issues a signed JWT
- `POST /runtime/nodes/{id}/rotate-token` — token rotation with JTI revocation
- `edgeplaned register` subcommand — registers a node and persists identity to
  `/etc/edgeplane/node.json`
- `edgeplane agent node join-token create/get/rotate` — admin CLI for managing
  node enrollment tokens
- `nodetoken` table for JTI-based revocation tracking
- edgeplaned reads node JWT from disk at startup — no env var injection needed

### CLI schema & federated discovery

- `edgeplane cli-schema` — emits a versioned JSON contract (`schema_version: 1`)
  describing the full CLI surface. Any consumer can parse this to discover
  available commands without hardcoded knowledge of the CLI structure.

### Security

- **OIDC hardening:** id_token claims now verified via the provider's userinfo
  endpoint instead of unverified JWT parsing; `redirect_path` sanitized to
  prevent open redirects; user-controlled values HTML-escaped in success pages
- **Proxy auth forwarding:** the `EP_API_PROXY` fallback now forwards the
  `Authorization` header to the backend
- **Bootstrap script hardened:** `EP_BASE_URL` and `EP_TOKEN` (pre-removal)
  changed from insecure defaults to required variables
- **Helm secrets:** sensitive values moved from ConfigMap to K8s Secret template;
  placeholder defaults cleared
- **CI hardening:** all GitHub Actions pinned to commit SHAs; `permissions:`
  blocks added to 4 workflows; `cargo audit` job added; Rust ecosystem added
  to Dependabot

### Documentation & site

- **edgeplane.ai launched** — custom landing page + full Starlight documentation
  site deployed via Cloudflare Pages
- Fixed 55 broken internal links (stale `/edgeplane/` path prefix)
- Standardized branding: Edgeplane → EdgePlane across all docs
- Rewrote auth documentation for the three active auth modes (OIDC, node JWT,
  service account)
- Fixed all stale CLI commands, socket paths, and env var references
- `curl -fsSL https://edgeplane.ai/install.sh | bash` install path with
  Cloudflare server-side redirect
- `MISSIONCONTROL_PHILOSOPHY.md` renamed to `PHILOSOPHY.md`

### Other changes

- Agent registration decoupled from Aria-specific conventions
- Source-agnostic agent listing and correct `StateDirSpec` serialization
- Additional test coverage for manifest import, idempotency, and unenroll

---

## [0.10.0] — 2026-05-23

Initial release under the **edgeplane** name. Forked from
[`RyanMerlin/missioncontrol`](https://github.com/RyanMerlin/missioncontrol)
at commit `ceff9f1` (the post-rename baseline that landed the
Domain / Mission / Task entity vocabulary).

### Brand identity

- Binaries: `edgeplane` (CLI), `edgeplaned` (per-node daemon), `edgeplane-tower` (server)
- Environment namespace: `EP_*` (replaces the prior `MC_*` prefix)
- Paths: `~/.edgeplane/` for user state, `~/.ep/` shorthand
- Org: edgeplane.ai

### Architectural changes from the fork point

- Single-workspace Cargo layout — all 12 crates resolve dependencies through
  a unified workspace root (previously 3 sibling workspaces with independent
  lockfiles)
- Cruft removal: disabled CI workflow referencing a missing `backend/`
  directory; design-validation prototypes whose ideas have shipped in the
  real implementation; meta scripts left over from prior renames
- No backward-compat shims to MissionControl — environment variables, file
  paths, binary names, HTTP headers, and MCP tool argument fields are all
  edgeplane-native from line 1

### Inherited from MissionControl

The engineering substrate — entity model (Domain / Mission / Task),
governance, overlap detection, persistent agent sessions via ACP, MCP-native
tool surface, three-tier persistence (Postgres + S3 + Git) — is all
inherited at maturity. See the upstream [`RyanMerlin/missioncontrol`](https://github.com/RyanMerlin/missioncontrol)
repo (archived) for development history and architectural decision records
prior to the fork.

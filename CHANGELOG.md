# Changelog

All notable changes to edgeplane, edgeplaned, and edgeplane-tower are recorded here. The three binaries ship in lockstep — `/VERSION` is the source of truth and `scripts/set-version.sh <new>` bumps all three in one step.

This project follows semantic versioning where possible, but pre-1.0 minor bumps may include breaking changes when the cost of a major bump outweighs the signal value.

## [Unreleased]

### Changed

- **Auth token prefix rebrand: `mcs_` → `ep_` (breaking).** Session, service-account,
  and client-secret tokens now mint and validate with `ep_` / `ep_sa_` / `ep_cs_`
  prefixes, replacing the MissionControl-era `mcs_` / `mcs_sa_` / `mcs_cs_`. Clean
  cutover with no dual-accept window — tokens issued under the old prefix are no
  longer valid and must be re-issued (`edgeplane auth login`). No DB migration:
  validation is by full-token hash; the prefix only routes session-vs-SA-vs-CS.
- **`edgeplaned` env vars rebranded (clean cut):** `MCD_WORK_DIR` → `EP_WORK_DIR`,
  `MCD_CRON_FILE` → `EP_CRON_FILE`. The old `MCD_*` names are no longer read. Set
  the `EP_*` names if you previously overrode these (defaults are unchanged).

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

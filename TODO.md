# Edgeplane — Active TODO

Items are ordered by priority. Add new items at the top of the relevant section.

## Now (blocked or in-flight)

Nothing currently blocked.

## Next

- First-class CLI create commands: `edgeplane domain create`, `edgeplane mission create`,
  `edgeplane task create` — operators should not need to invoke MCP tools or run an agent
  to scaffold work. These are the human-facing counterparts to the MCP-only surface that
  currently handles all entity creation.

- NORTHSTAR as a real S3-backed file: `domains/{domain_id}/NORTHSTAR.md`. Currently
  `domain.northstar_md` is an inline Postgres column — the intent (per
  `docs/schema-packs/NORTHSTAR.example.md`) was always a first-class document stored in
  object storage. Migration: add `northstar_s3_path` + `northstar_sha256` columns, write
  NORTHSTAR.md to S3 on domain create/update, expose `GET/PUT /domains/{id}/northstar`
  endpoints, add `get_domain_northstar` MCP tool so agents load it on join (same pattern
  as CLAUDE.md). Same applies to Mission: `workstream_md` → `WORKSTREAM.md` in S3.

- MCP audit: tag all ~65 MCP tools as `protocol-only` (agent runtime, stays MCP) or
  `needs-cli-equivalent` (management op, gets a CLI command). See ADR 0005.

## Backlog

- `edgeplane tui` SSE agent-feed: verify end-to-end against a live cluster (proxy fix
  shipped, needs integration test with a real SSE-emitting backend).

## Hardening (OSS readiness)

Portability and deployment hardening so EdgePlane runs cleanly beyond a single developer's
machine. Roughly dependency-ordered; each is a self-contained piece of work.

- **[in progress] XDG-aware paths.** Make path resolution honor `$XDG_CONFIG_HOME` /
  `$XDG_STATE_HOME` / `$XDG_DATA_HOME` / `$XDG_RUNTIME_DIR`, with a unified `~/.edgeplane`
  default and `EP_HOME` override. Collapse the duplicated home-dir logic into the single
  `edgeplaned-paths` crate (currently reimplemented in `edgeplaned-sync` and `edgeplane-tower`,
  where it has drifted — the tower copy falls back to `/root/.edgeplane`).
- **System-install mode + service account.** Support a non-login `edgeplane` system user with
  `/etc/edgeplane`, `/var/lib/edgeplane`, `/run/edgeplane`, and a hardened **system**
  `edgeplaned.service` (`DynamicUser=`/`ProtectSystem=strict`) alongside the `--user` dev unit.
- **Distribution packaging.** Container + Helm chart + systemd unit template + an
  `edgeplane init` scaffolder. Reproducible, versioned, signed — no "build from my checkout."
  (Includes fixing the node-enrollment snippet to use the correct install script + join flags.)
- **Pluggable secrets backend.** Abstract secret resolution behind an interface: env / file /
  Vault / Kubernetes Secrets / cloud secret managers. No backend baked in.
- **Decouple runtime from the terminal multiplexer.** `edgeplaned` / the ACP supervisor owns
  session lifecycle; zellij/tmux becomes an optional dev-attach UI, not a hard dependency for
  headless runs.
- **Config precedence chain.** A documented layered config: flags > env > project > user >
  system > built-in defaults.
- **Multi-tenancy formalization.** Per-tenant state dirs, secret scoping, and resource quotas
  on the existing domain/agent model. (Domain authorization itself already enforces default-deny
  as of v0.15.x.)

## Done (recent)

- [x] `crates/edgeplaned/scripts/demo_three_agents.sh`: end-to-end dependency chain
      demo (mission + kluster + 3 tasks + claim/complete simulation via REST API);
      `test_work.rs`: 5 tests covering broadcast isolation, route registration (2026-05-09)
- [x] CI updated: `test_proxy` and `test_work` added to `rust-test` job (2026-05-09)
- [x] `edgeplane-tower` `--api-proxy` / `EP_API_PROXY` CLI flag exposed (2026-05-09)
- [x] edgeplaned work loop: adaptive backoff (5s→30s), `depends_on`/`produces`/`consumes`
      in `MeshTaskRecord` + `TaskSpec`, consumes-gate in `filter_eligible`, WS notify
      endpoint on controlplane (`/work/agents/{id}/notify`), WS client in daemon with
      exponential reconnect backoff, `wake_rx` replaces fixed sleep on no-task (2026-05-09)
- [x] `edgeplane-tower` proxy fallback: `api_proxy` field in `AppConfig`/`AppState`,
      fallback handler forwards unknown routes → upstream, returns 502 on failure;
      `test_proxy.rs` tests pass (2026-05-09)
- [x] `edgeplaned` node heartbeat: `node_id` in `DaemonConfig`; background task sends
      periodic heartbeats to `/runtime/nodes/{id}/heartbeat` with Tailscale IP/FQDN
      from `MachineInfo::detect()` (2026-05-09)
- [x] `edgeplane secrets infisical` Universal Auth: `client_id + client_secret → token exchange`
      fully implemented in `edgeplaned-secrets/src/client.rs` with in-process token cache (2026-04-28)
- [x] Approval flow wiring: `POST /klusters/{id}/approvals/{approval_id}/respond` in
      `edgeplane-tower`, TUI key handlers dispatch `WorkRequest::RespondApproval` (2026-04-28)
- [x] Tailscale detection: `MachineInfo::detect()` runs `tailscale ip --4` and
      `tailscale status --json`; fields propagate through node register/heartbeat API (2026-05-09)
- [x] `edgeplane-tower` renamed from `edgeplane-server` — dir, package, binary, lib, CI,
      Dockerfiles, docs (2026-05-09)
- [x] `edgeplane tui` P0–P5: skeleton, work pool, mission-matrix, approval-queue, receipts,
      agent-feed SSE, secrets browser, multi-profile Infisical lift (2026-04-28)
- [x] `edgeplane secrets infisical {add,list,use,test,rm,get}` CLI (2026-04-28)
- [x] `edgeplaned` secrets broker: SessionStore + SecretsGateway Unix socket +
      CapabilityDispatcher broker mode + `edgeplaned get-secret` helper (2026-04-28)
- [x] `edgeplane-tower` GET /raft/status endpoint (2026-04-28)
- [x] `edgeplane-tower` SSE proxy fix: header forwarding + streaming response body (2026-04-28)
- [x] `edgeplane tui` status bar wired to /raft/status: shows `node N · role · connected Xms` (2026-04-28)

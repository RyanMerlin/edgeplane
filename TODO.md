# MissionControl — Active TODO

Items are ordered by priority. Add new items at the top of the relevant section.

## Now (blocked or in-flight)

Nothing currently blocked.

## Next


## Backlog

- `mc tui` SSE agent-feed: verify end-to-end against a live cluster (proxy fix
  shipped, needs integration test with a real SSE-emitting backend).

## Done (recent)

- [x] `integrations/mcd/scripts/demo_three_agents.sh`: end-to-end dependency chain
      demo (mission + kluster + 3 tasks + claim/complete simulation via REST API);
      `test_work.rs`: 5 tests covering broadcast isolation, route registration (2026-05-09)
- [x] CI updated: `test_proxy` and `test_work` added to `rust-test` job (2026-05-09)
- [x] `mc-controlplane` `--api-proxy` / `MC_API_PROXY` CLI flag exposed (2026-05-09)
- [x] mcd work loop: adaptive backoff (5s→30s), `depends_on`/`produces`/`consumes`
      in `MeshTaskRecord` + `TaskSpec`, consumes-gate in `filter_eligible`, WS notify
      endpoint on controlplane (`/work/agents/{id}/notify`), WS client in daemon with
      exponential reconnect backoff, `wake_rx` replaces fixed sleep on no-task (2026-05-09)
- [x] `mc-controlplane` proxy fallback: `api_proxy` field in `AppConfig`/`AppState`,
      fallback handler forwards unknown routes → upstream, returns 502 on failure;
      `test_proxy.rs` tests pass (2026-05-09)
- [x] `mcd` node heartbeat: `node_id` in `DaemonConfig`; background task sends
      periodic heartbeats to `/runtime/nodes/{id}/heartbeat` with Tailscale IP/FQDN
      from `MachineInfo::detect()` (2026-05-09)
- [x] `mc secrets infisical` Universal Auth: `client_id + client_secret → token exchange`
      fully implemented in `mcd-secrets/src/client.rs` with in-process token cache (2026-04-28)
- [x] Approval flow wiring: `POST /klusters/{id}/approvals/{approval_id}/respond` in
      `mc-controlplane`, TUI key handlers dispatch `WorkRequest::RespondApproval` (2026-04-28)
- [x] Tailscale detection: `MachineInfo::detect()` runs `tailscale ip --4` and
      `tailscale status --json`; fields propagate through node register/heartbeat API (2026-05-09)
- [x] `mc-controlplane` renamed from `mc-server` — dir, package, binary, lib, CI,
      Dockerfiles, docs (2026-05-09)
- [x] `mc tui` P0–P5: skeleton, work pool, mission-matrix, approval-queue, receipts,
      agent-feed SSE, secrets browser, multi-profile Infisical lift (2026-04-28)
- [x] `mc secrets infisical {add,list,use,test,rm,get}` CLI (2026-04-28)
- [x] `mcd` secrets broker: SessionStore + SecretsGateway Unix socket +
      CapabilityDispatcher broker mode + `mcd get-secret` helper (2026-04-28)
- [x] `mc-controlplane` GET /raft/status endpoint (2026-04-28)
- [x] `mc-controlplane` SSE proxy fix: header forwarding + streaming response body (2026-04-28)
- [x] `mc tui` status bar wired to /raft/status: shows `node N · role · connected Xms` (2026-04-28)

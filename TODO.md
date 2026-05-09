# MissionControl — Active TODO

Items are ordered by priority. Add new items at the top of the relevant section.

## Now (blocked or in-flight)

Nothing currently blocked.

## Next

### Approval flow wiring
- **What:** Wire the `y/n/e/s` key handlers in `mc tui`'s approval-queue screen to
  actual API calls. Currently the screen renders pending approvals but taking action
  does nothing.
- **Backend needed:** `POST /klusters/{id}/approvals/{approval_id}/respond` in
  `mc-controlplane` (or proxy to Python backend). Response body: `{"ok":true}`.
- **TUI needed:** `ApprovalQueueState::handle_key` dispatches a
  `WorkRequest::RespondApproval` on `y`/`n`; result re-fetches the queue.
- **Estimate:** ~half day.

### mc-mesh work loop deepening
- **What:** Richer task dispatch: watch WebSocket, dependency enforcement between
  tasks, dispatch loop improvements, 3-agent demo scenario.
- **Design:** Described in `~/.claude/plans/` (`project_mesh_workloop.md`). Needs a
  new plan session to turn the design into an implementation plan.
- **Estimate:** Separate plan, 2–3 days of implementation.

## Backlog

- Fix parallel test flake in `integrations/mc/tests/test_discover.rs` — two tests
  write to the same temp path and race. Add unique temp dir per test.
- `mc tui` SSE agent-feed: verify end-to-end against a live cluster (proxy fix
  shipped, needs integration test with a real SSE-emitting backend).
- Raft consensus implementation behind the `--serve` flag in `mc-controlplane` (currently
  the server is standalone; the flag parses but does nothing).
- `mc secrets infisical` Universal Auth (client_id + client_secret → token exchange)
  — the config fields exist but `InfisicalClient` only uses `service_token` today.

## Done (recent)

- [x] `mc tui` P0–P5: skeleton, work pool, mission-matrix, approval-queue, receipts,
      agent-feed SSE, secrets browser, multi-profile Infisical lift (2026-04-28)
- [x] `mc secrets infisical {add,list,use,test,rm,get}` CLI (2026-04-28)
- [x] `mc-mesh` secrets broker: SessionStore + SecretsGateway Unix socket +
      CapabilityDispatcher broker mode + `mc-mesh get-secret` helper (2026-04-28)
- [x] `mc-controlplane` GET /raft/status endpoint (2026-04-28)
- [x] `mc-controlplane` SSE proxy fix: header forwarding + streaming response body (2026-04-28)
- [x] `mc tui` status bar wired to /raft/status: shows `node N · role · connected Xms` (2026-04-28)

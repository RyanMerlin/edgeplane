# EP-1: Durable, Fenced Dispatch + Supervised Lease-Aware Task Worker

**Status:** draft, pending Merlin's review
**Effort:** L (correctness + reliability)
**Entities involved:** Task (`entities.md` § Task), AgentRun (`entities.md` § AgentRun), MeshAgent

## Why

`Task` rows with `kind='claimable'` are dispatched via a claim/lease/heartbeat protocol (migration 0014). The lease machinery exists in schema (`claim_lease_id`, `lease_expires_at`, `attempt`/`max_attempts`) but is only half-enforced: several mutation endpoints don't atomically check the lease against the row they're updating, two daemon-side task-consuming loops diverge sharply in lease discipline, and neither loop survives a crash or a graceful restart cleanly. The result is a real, reproducible fencing violation: a long-running ephemeral subagent whose lease expires and gets reclaimed by another claimer can still complete/fail the task out from under the new claimer when its subprocess finally exits, because it never carried a lease token to be checked against in the first place.

This spec was produced through: reading `entities.md` § Task (the existing "Fencing" note already documents the claim-reclaim gap this closes), direct source verification via CodeGraph against the current `edgeplane-tower` and `edgeplaned` crates (not the originating vault survey's summary alone), and two independent adversarial passes by Codex (gpt-5.5, xhigh) — the second of which found two outright blockers in the first draft of this spec and widened its scope. Findings below are annotated with where they came from; nothing here is asserted without a file:line check against current source.

## Scope

Originally scoped narrowly (claim/heartbeat/complete/fail on the two daemon loops). Widened after the second Codex pass surfaced that `cancel`, `block`, `unblock`, `resolve_gate`, and `progress` share the identical fencing gap — some more severely (`block_task` today has **no** lease or status precondition at all; `resolve_gate` can finish/fail a task **without clearing** `claim_lease_id`/`claimed_by_agent_id`, letting a stale claimer's proof remain valid past what should be a terminal state). Decision: fold the full task-lifecycle surface into EP-1 rather than ship a fence with known holes in it.

**In scope:** tower-side atomic fencing across every mutating task-lifecycle endpoint (REST + MCP), the daemon-side `task_worker.rs`/`task_loop.rs` unification and lease-discipline fixes, and crash-restart supervision + drain for both daemon loops.

**Out of scope:** the durable handoff/reassignment transition log and the approval-gate-distinct-from-ownership concept, both already flagged in `entities.md` § Task as deferred, real follow-on work — EP-1 does not attempt either.

## 1. Tower: atomic fencing on every mutating task endpoint

### The pattern to converge on

`claim_task` (`work.rs:1004`) already does this correctly for the exclusive-claim path: a transaction, `SELECT ... FOR UPDATE SKIP LOCKED`, a CAS `UPDATE ... WHERE id=$1 AND version_counter=$N`, and on `Ok(None)` returns a `409` via the existing `conflict()` helper. Every other mutating endpoint instead does SELECT → validate in application code → blind `UPDATE ... WHERE id=$1` — a TOCTOU window between the check and the write.

Target pattern for `heartbeat_task`, `complete_task`, `fail_task`, `cancel_task`, `block_task`, `unblock_task`, `resolve_gate`, and `append_progress`: **the lease/ownership/status check moves into the `UPDATE`'s `WHERE` clause**, and the response code is derived from *whether the fenced update returned a row*, not from an earlier precondition check. Concretely:

```sql
-- kind='claimable' rows:
UPDATE task SET ... WHERE id=$1
  AND kind='claimable'
  AND status IN (<legal source states for this transition>)
  AND claim_lease_id = $lease
  AND lease_expires_at >= now()          -- exact-token match alone is not enough (see below)
RETURNING *
```

`kind='assigned'` rows keep their existing owner/full-trust predicate (unchanged) — **do not** add `kind='claimable'` unconditionally to any of these queries. The first draft of this spec did exactly that on `complete_task`/`fail_task`, which are deliberately unified across both kinds today (Codex catch: this would have silently broken assigned-task completion — a blocker, not a nitpick).

### Why the lease-expiry check is required, not just token equality

`claim_lease_id` is currently only cleared by `expire_stale_leases`. Between the moment a lease's `lease_expires_at` passes and the next time the sweep actually runs, the *token itself* is still present and would pass a pure `claim_lease_id = $lease` check even though the 120s window has elapsed. If the lease is meant to be authoritative at 120s, the predicate must also assert `lease_expires_at >= now()` in the same atomic statement — this was the second blocker Codex found in the first draft.

### 403 vs 409, done correctly

`authz_task_owner` currently returns `403` for *any* mismatch — true non-owner and stale-lease alike — and runs as a precheck *before* the mutating query. That means simply relabeling "403 → 409" at the response layer doesn't work: the precheck would still fire 403 for a stale lease before the fenced update is ever attempted.

Converge on the `claim_task` pattern instead: don't precheck ownership as a gate. Run the fenced `UPDATE` directly (still behind an `authz_domain` check for coarse access control — that part is fine as-is). If it returns zero rows, re-fetch the row to distinguish the reason: `404` if the task no longer exists, `409` if it exists but the predicate failed (wrong status, wrong lease, or wrong owner-with-a-lease-supplied), `403` only for the true case of "no ownership proof presented at all and not full-trust/admin." This mirrors `claim_task`'s existing `conflict()`-on-`None` handling rather than inventing a new pattern.

**Addendum (dual-review, 2026-08-19/20): "wrong lease" deliberately means *any* presented lease value, not just one that matches the row's current `claim_lease_id`.** Two independent adversarial reviewers flagged this as letting a caller suppress the 403 signal — attach any string and get 409 instead. Considered and rejected requiring an exact match: `expire_stale_leases` clears a reclaimed row's `claim_lease_id` to `NULL`, so a legitimate agent presenting its own, once-real, since-reclaimed lease would get 403 (a hard error client-side, per `edgeplaned-work`'s status-code mapping) instead of 409 (a graceful lose-the-race path) under strict matching — exactly the caller this classifier exists to be generous to, not the caller it exists to catch. `classify_fenced_rejection` is diagnostic-only (the real authorization decision is already atomic, made by the fenced `UPDATE` before this function ever runs), so the HTTP status code was never the right place for an abuse-detection signal regardless. That signal now lives in a structured `fenced_rejection` tracing event emitted alongside the classification, which does distinguish a lease that matches the row's current value from one that never did — the thing a status code alone can't do without breaking the reclaim-race case.

### The specific transitions and their legal source states

| Endpoint | Legal source states (`kind='claimable'`) | Notes |
|---|---|---|
| `heartbeat_task` | `claimed`, `running` | |
| `complete_task` | `claimed`, `running`, `waiting_review` | First spec draft wrongly narrowed this to `claimed`/`running` — Codex catch: `waiting_review` is a legitimate source state for review-gated tasks. |
| `fail_task` | mirror `complete_task`'s set — verify exact current precondition against `work.rs` at implementation time | |
| `cancel_task` | any non-terminal | already calls `authz_task_owner`; needs the same fenced-CAS treatment, not new authz logic |
| `block_task` | `claimed`, `running` | **Currently has no precondition at all** (Codex finding) — this is net-new, not a hardening of an existing check |
| `unblock_task` | `blocked` (or whatever the block transition sets) | verify against current schema/status vocabulary at implementation time |
| `resolve_gate` | — | See its own subsection below — it needs a bespoke transaction, not the generic predicate. |
| `append_progress` (REST) | `claimed`, `running` | REST progress currently has **no lease field at all**. Add one, required, checked in the same insert (a transaction: verify current lease/status, then insert — not a separate precheck, to avoid reintroducing the exact TOCTOU being fixed elsewhere). |

The pending-gates branch inside `complete_task` gets its own fix: today it does an *earlier*, separate, unfenced `UPDATE ... WHERE id=$1` to set `waiting_review` before the final completion update — and a gate can be created in the window between the pending-gate `SELECT` and that update (a second, independent race Codex found). Fold both updates into one transaction: fence the pending-gate check and the resulting transition (to either `waiting_review` or `finished`) together, e.g. via a CTE that computes gate existence and performs one fenced transition.

### Correction: terminal transitions don't fully clear ownership today (third review pass)

The first two revisions of this spec claimed `resolve_gate` should clear claim fields "matching what `complete_task` already does." **That's false** — verified against source. `complete_task`/`fail_task` clear `lease_expires_at` and `claim_lease_id` on their terminal `UPDATE`, but leave `claimed_by_agent_id` behind (`work.rs:1474`, `work.rs:1571`). Since `authz_task_owner` accepts `claimed_by_agent_id` alone as ownership proof (independent of a live lease), this is itself a residual gap in the *existing* endpoints, not just something `resolve_gate` needs to newly implement. Fix: every terminal transition (`complete_task`, `fail_task`, and `resolve_gate`) clears `claim_lease_id`, `lease_expires_at`, **and** `claimed_by_agent_id` together.

### `resolve_gate`: bespoke transaction, not the generic predicate

`resolve_gate` can't literally reuse the claim-lease predicate above — the caller resolving the gate is the *gate's* owner, not necessarily the task's current lease holder. It needs its own transaction: a fenced `reviewgate` update keyed on `status='pending'` + gate ownership, then a fenced task transition out of `waiting_review` that recomputes remaining-gate state in the same transaction (so a second gate created concurrently isn't missed). The current rejected-gate path clears none of the task's lease fields at all (`work.rs:2017`, `work.rs:2024`) — needs the same three-field clear as above.

### `block`/`unblock`: lease retention is an undecided design question, not an oversight

Today, `block_task` keeps all claim fields when it mutates a task to `blocked`, and `unblock_task` sets status back to `ready` without clearing them either (`work.rs:1623`, `work.rs:1745`). Before fencing these, the spec needs to actually decide: does blocking release the claimer's lease (so a different agent can pick the task back up if the original claimer never returns), or does it preserve the claim (so only the original claimer can unblock/resume it)? This spec proposes: **block releases the lease** (clears `claim_lease_id`/`lease_expires_at`/`claimed_by_agent_id`, same as a terminal transition) — a blocked task re-enters the claimable pool rather than pinning it to a claimer that may never come back. `block_task` also currently has ownership auth but no lease-or-status precondition on the mutation itself (`work.rs:1616`, `work.rs:1623`) — add the fenced predicate regardless of the lease-retention answer.

### Broadcast claims and full-trust/admin bypass — explicitly in scope, explicitly unfenced by design

`claim_task`'s broadcast branch (no `FOR UPDATE`/CAS, by design — broadcast tasks are meant to be claimable by multiple agents) is still a blind `UPDATE ... WHERE id=$1` after only a precheck (`work.rs:1068`). This is intentional for broadcast semantics, but should be an explicit, stated exception in this spec rather than something that reads as an oversight once every other endpoint is fenced.

Separately: `authz_task_owner` today lets full-trust/admin principals bypass ownership entirely. The fenced predicates above require an exact live-lease match for `kind='claimable'` rows — that would silently break legitimate full-trust/admin operations (e.g. an operator force-completing a stuck task) unless the predicate explicitly carries an admin/full-trust bypass branch. State it: the fenced `UPDATE` becomes `WHERE ... AND (claim_lease_id = $lease OR $is_full_trust_or_admin)`.

### MCP mirrors the same gap independently

`routes/mcp.rs` has its own direct-SQL complete/fail/block handlers (a second code path entirely, not a thin wrapper over the REST handlers) with the identical unfenced-update pattern, including the same `block_mesh_task` precondition gap. These get the same fix. Note MCP responds with a JSON `ok`/`error` shape, not HTTP status codes — the REST 403→409 change does **not** automatically extend to MCP; the CLI parses MCP's JSON contract directly (`commands.rs:4089`), so MCP's error classification needs its own explicit (if parallel) treatment, not an assumption that fixing REST covers it.

**Two more specifics, found by dual-review (2026-08-19/20) while Tasks 1-3 were landing — confirm still true when this section is implemented, don't assume they've drifted:** (1) `mcp.rs`'s handlers still write `updated_at=NOW()` — the exact timezone-GUC dependency Task 1 fixed on the REST side (binds a Rust-computed naive `now` instead), not yet ported here. (2) `heartbeat_mesh_task`'s lease TTL is 300s against REST's `LEASE_TTL_SECS = 120s` — the two paths can put the same row in different freshness states depending on which one last touched it; needs a decision (unify the constant, or document why they're allowed to differ) as part of this section's implementation, not left as an unstated inconsistency.

### Background expiry sweep

`expire_stale_leases` currently only runs as a side effect of `list_tasks` (`work.rs:658`) — if nothing polls, expired leases never requeue. Add an independent periodic sweep (tower-side `tokio::time::interval`, every 30s, across all missions in one unscoped query rather than looping per-mission).

Two things the first draft missed:
- The helper currently swallows DB errors silently (`work.rs:496`, `let _ = ... .execute(db).await`). A background sweep that fails silently would pass every test and still leave stuck leases in production. Log and (once EP-2's observability backbone lands) surface a metric on sweep failures.
- The only supporting index is a partial index on `status='claimed'`, but heartbeats transition a task to `status='running'` — an every-30s all-mission scan needs an index that covers both `claimed` and `running` rows with a non-null, past-due `lease_expires_at`, or it becomes a real load source at scale rather than a cheap sweep.

Double-firing (the per-request call in `list_tasks` racing the new periodic sweep) is safe: both are guarded by the same `lease_expires_at < $1` predicate and Postgres re-checks `UPDATE` predicates after any row-lock wait, so a lease claimed in the gap between two sweeps can't be reclaimed by a sweep that started before the claim.

## 2. Daemon: `task_loop.rs` fixes

Two bugs, not one:

1. **Quiet-stream heartbeat gap.** `stream_and_heartbeat` (`task_loop.rs:316`) only heartbeats *after* a progress event arrives (`stream.next().await` gates it) — a quiet stream can miss the 120s window entirely. Fix: `select!` between `stream.next()` and an independent `tokio::time::interval` ticker, heartbeating on schedule regardless of stream activity.
2. **Lease-loss is miscategorized as an ordinary failure.** On `LeaseMismatch`, `stream_and_heartbeat` returns `Ok(false)` (`task_loop.rs:340`), and the caller (`task_loop.rs:291`) treats `false` identically to "the agent reported failure" and calls `fail_task` — itself now a lease-less-ish call into a task that isn't this loop's anymore. Fix: give `stream_and_heartbeat` a third outcome (e.g. `Completed(bool) | LeaseLost`), and skip complete/fail entirely on `LeaseLost` — the task already belongs to whoever reclaimed it.

Also apply `task_worker.rs`'s "missing lease after claim is fatal" rule here too: `task_loop.rs` currently accepts `Option<&str>` for the lease and proceeds even when it's `None` (`task_loop.rs:192`). Abort before injecting the task if the claim response carried no lease.

Two more, found on the third review pass:

- **`post_progress` doesn't carry a lease today.** If `append_progress` becomes lease-required per the tower section above, `edgeplaned_work::task::post_progress` (`task.rs:106`) needs to actually send `claim_lease_id` — right now it sends none, so `task_loop.rs`'s progress-forwarding calls would start failing the moment the tower side requires one.
- **Dead code found, adjacent enough to fold in:** `current_task_id`/`current_lease_id` (`task_loop.rs:83`) are declared specifically to let the watchdog's strict/autonomous offline policy fail an in-flight task — but nothing in the loop ever actually sets them. The offline-fail path has silently never worked. Since this is the same lease-tracking surface EP-1 is already touching, wire these up rather than leaving a second, unrelated dead lease-discipline bug next to the one being fixed.

## 3. Daemon: `task_worker.rs` unification

**Not** a literal "call `task_loop`'s functions" swap — two real constraints found by the first Codex pass:

- **Keep task_worker's own routing.** The shared `claim::try_claim_one` filters only by capability and has no concept of `claim_policy.target_profile`, which is `task_worker`'s actual dispatch rule. `scan_ready_tasks`/`should_claim` stay as-is; only the HTTP calls for claim/heartbeat/complete/fail get replaced.
- **Two `BackendClient`s, not one agent-scoped swap.** Task lifecycle calls must run as the ephemeral MeshAgent's own identity (the shared `claim_task` helper posts an empty body, and the tower only honors an explicit `agent_id` override for full-trust/admin — under the daemon's own token, that's the *node*, not the ephemeral agent, which is why `task_worker` today explicitly sends `{"agent_id": agent_id}`). But `DELETE /work/agents/{id}` is authorized by `enrolled_by_subject` — the daemon/session principal that did the enrolling, not the ephemeral agent itself (second Codex pass). So: a daemon-scoped client for enroll/delete (unchanged), a fresh agent-scoped `BackendClient::new(base_url, agent_token)` (not a `.clone()` with `set_token` — clones share the token lock) for claim/heartbeat/complete/fail.

**Heartbeat during the blocking subprocess.** `cmd.output().await` blocks with no progress stream to piggyback a heartbeat on. Pattern (from the first Codex pass, concrete):

```rust
async fn run_child_with_lease(
    mut cmd: tokio::process::Command,
    client: &BackendClient,
    task_id: &str,
    lease_id: &str,
) -> Result<std::process::Output, TaskError> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let child = cmd.spawn().map_err(|e| TaskError::Other(anyhow!(e)))?;
    let mut output = Box::pin(child.wait_with_output());
    let mut hb = interval_at(Instant::now() + HEARTBEAT_INTERVAL, HEARTBEAT_INTERVAL);
    loop {
        tokio::select! {
            result = &mut output => return result.map_err(|e| TaskError::Other(anyhow!(e))),
            _ = hb.tick() => match task::heartbeat_task(client, task_id, Some(lease_id)).await {
                Ok(()) => {}
                Err(TaskError::LeaseMismatch) => return Err(TaskError::LeaseMismatch), // drops `output`, kills child via kill_on_drop
                Err(TaskError::Other(e)) => tracing::warn!("heartbeat failed: {e:#}"),
            }
        }
    }
}
```

On `LeaseMismatch`, the caller does **not** call complete/fail afterward — the task isn't task_worker's anymore.

**Additional fixes bundled here:**
- Missing `claim_lease_id` after claim is fatal — abort before spawning.
- **The agent-scoped client must copy `api_prefix`.** The daemon's own client is constructed with an `/api` prefix (`daemon.rs:200`); a naive `BackendClient::new(base_url, agent_token)` for the agent-scoped client would drop that and call the wrong URLs. Copy the daemon client's `api_prefix` into the new client explicitly.
- **Missing `agent_token` at enrollment must become fatal, not a silent fallback.** Today, when enrollment doesn't return an `agent_token`, `task_worker.rs` falls back to inheriting the daemon's own token (`task_worker.rs:367`) — a reasonable graceful-degradation choice under the *old* architecture. Under EP-1's two-client identity model, that fallback would silently defeat the whole point of agent-scoped task lifecycle calls (the "claim identity" bug this spec exists to fix). Treat a missing `agent_token` as fatal: abort the lifecycle before claiming.
- **In-flight-set cleanup, corrected.** The first draft proposed a "drop-guard"; that doesn't work — a `Drop` impl can't `.await` a `tokio::Mutex` lock (second Codex pass caught this). Use `futures::FutureExt::catch_unwind` around the `run_task_lifecycle` call (`AssertUnwindSafe`), so the in-flight removal runs unconditionally afterward regardless of panic.
- **AgentRun accounting.** The failure path currently POSTs `{"status":"failed"}` to `/runs/{id}/complete`, which unconditionally marks the run `completed` (`runs.rs:159`) — every failed subagent run is silently logged as a success today, independent of this migration. Switch failure/lease-loss paths to `/runs/{id}/fail`.

## 4. Supervision + drain

**Supervision.** Wrap both `task_loop::run_for_agent` and `task_worker::run`'s spawn sites in a backoff-restart loop modeled on `acp_session_supervisor::run_for_agent`'s pattern (catch error/panic, exponential backoff 2s→60s, reset after a stable-runtime threshold) — but not a blind copy. Two things the copy would get wrong (second Codex pass):
- `task_worker::run` has a legitimate clean-exit path (`task_worker_enabled = false` → returns immediately). The wrapper must distinguish "exited because it's disabled" from "crashed" — restarting a deliberately-disabled worker in a loop is its own bug.
- Backoff-forever with no circuit breaker can mask a genuinely fatal misconfiguration (bad DB connection string, poisoned state) behind an endless, silent restart loop. Add a crash-count-within-window threshold that, once exceeded, stops restarting and surfaces a clearly-logged terminal state rather than restart-forever.

**Drain.** Corrected from the first draft, which conflated this with EP-2's tower-side graceful HTTP shutdown (`edgeplane-tower/src/main.rs:91`) — a completely separate process. `edgeplaned` has Ctrl-C handling for other cleanup today but **no SIGTERM handler at all**, and `task_worker.rs`'s own doc comment already says graceful shutdown isn't implemented. This is genuinely net-new plumbing: a Unix SIGTERM handler in `edgeplaned`, a shared cancellation flag (`tokio_util::sync::CancellationToken`) threaded into both loops. In-flight tasks are allowed to finish or lapse — the lease TTL plus the now-independent background sweep already reclaims abandoned work, so drain doesn't need bespoke wait-for-completion logic.

Third review pass found the drain design as first written was underspecified in three ways:

1. **"Check a flag at the top of each poll iteration" isn't responsive enough.** The `sleep`/`select!` wait points inside each loop's idle path also need to observe the cancellation token directly (race the token against the sleep/select, not just check-before-looping), or a loop mid-sleep won't notice shutdown until its next full poll interval.
2. **Restarting `task_loop::run_for_agent` needs to account for what it spawns internally.** The function isn't self-contained — it detaches its own message-relay loop and notify-WebSocket listener (`task_loop.rs:60`, `task_loop.rs:78`) as separate `tokio::spawn` calls tied to the *call*, not the logical agent-loop lifetime. A supervisor that restarts the outer function on crash, without explicitly cancelling those inner spawns first, leaves the old relay/listener tasks running forever while a new restart spawns fresh ones — an accumulating leak on every crash-restart cycle. The supervisor needs to own and cancel those inner tasks as part of what "restart" means, not just re-call the outer function.
3. **In-flight `task_worker` subprocesses need an explicit disposition during the drain window**, not just "let it lapse." Since these are real child processes (`claude -p`), the design should state whether drain waits for them up to the grace period, or kills them via the same `kill_on_drop` mechanism used for lease-loss. This spec proposes: wait up to the configured grace period, then kill — consistent with treating drain as "stop claiming new work, then behave like a lease-loss event for anything still running when time's up."

The daemon's actual termination grace period (systemd/k8s `TimeoutStopSec`/`terminationGracePeriodSeconds`) must be an explicit configured value, not an assumed default — set it as part of implementation and reference it from the drain logic above rather than hardcoding a duration.

## Testing

- Tower: extend `test_task_kind_unification.rs`'s reclaim test to assert the fenced CAS rejects a stale-lease request (was pinned to 403, becomes 409 via the re-fetch-and-classify pattern); add coverage for `waiting_review`-sourced completion, kind='assigned' completion still working after the predicate split, `block_task`'s new precondition, `resolve_gate` clearing the lease fields, and a concurrent-heartbeat-during-reclaim race test.
- Tower: a test asserting the background sweep logs (not swallows) a DB error, and that heartbeat-set `running` rows are actually covered by whatever index backs the sweep.
- Daemon: unit test for `stream_and_heartbeat`'s new `LeaseLost` outcome skipping complete/fail; unit test for the `catch_unwind`-wrapped in-flight cleanup (inject a panic, assert the set is clean afterward); test for the two-client split (assert agent-scoped calls carry the ephemeral agent's token, not the daemon's); test for the supervision wrapper's clean-exit-vs-crash distinction and circuit breaker.
- End-to-end: a harness that deliberately starves a lease past TTL while a `task_worker` subprocess is still running, confirming the original reclaim-then-original-completes-anyway bug is actually closed.

## Resolved during the third review pass (were open items in earlier drafts)

- **`fail_task`'s current precondition**, confirmed against source (`work.rs:1544`): `claimed`/`running`/`waiting_review` for `kind='claimable'`, any non-terminal status for `kind='assigned'` — identical to `complete_task`'s set, as this spec had guessed but not verified. The fenced predicate table above can be taken as confirmed for `fail_task`.
- **`unblock_task`'s current precondition**, confirmed against source: there isn't one today — no source-status guard at all, it writes `status='ready'` unconditionally (`work.rs:1745`). The only "blocked" status literal in source is `'blocked'` (`work.rs:1623`), confirming the table entry above.

## Open items for implementation time (not blocking spec approval, but not yet resolved here)

- The daemon's systemd/k8s termination grace period isn't yet confirmed — needs an explicit configured value before the drain design (§4) is implemented; not blocking the tower-fencing work (§1), which has no dependency on it.

# Shared Fenced-Transition Primitive — Design

**Status:** approved by Merlin (design shape + scope), pending spec review before implementation plan
**Effort:** M (refactor of 5 already-shipped REST endpoints + net-new MCP fencing)
**Entities involved:** Task (`entities.md` § Task)

## Why

EP-1's Tasks 1-8 fenced eight REST task-lifecycle endpoints in `crates/edgeplane-tower/src/routes/work.rs`, each hand-deriving its own atomic-CAS SQL predicate independently. This duplication is not cosmetic — it is the direct cause of a CRITICAL bug: Task 8's `append_progress` reintroduced the exact broadcast-ownership-bypass shape commit `37dca61a` had already fixed elsewhere on this same branch, because the implementing session copied a stale plan-document predicate instead of cross-checking already-fixed sibling code (commit `82507f12`).

A follow-up security review then found MCP's mirror of these endpoints (`routes/mcp.rs`) — which hand-derives its *own*, separately-written, less rigorous copies of similar logic — has a HIGH-severity live-reproduced gap: `heartbeat_mesh_task` has no freshness check at all and can indefinitely revive an expired lease through the same `task.lease_expires_at` column REST's fencing reads, undoing the REST-side work for anyone using MCP (which this repo's own docs call the primary mesh-task path, and which `edgeplane task mesh <verb>` — a real, fully-wired CLI, not a stub — calls under the hood via `POST /mcp/call`).

Merlin's question, given two real bugs from the same root cause landed back to back: *"are we being efficient with the ownership checks? Is there a better central or standardized way to handle this?"* This spec is that design.

## Scope

**In scope — the five transitions with both a REST and an MCP surface:** heartbeat, complete, fail, block, progress. Both REST (`work.rs`) and MCP (`mcp.rs`) will call one shared primitive for these five; REST's five existing hand-rolled implementations are refactored to call it too, not just MCP's new fencing. Refactoring already-shipped code is deliberate, not incidental: if REST keeps its own separate implementation, the shared primitive is a *third* copy alongside REST's original and MCP's old one — the exact duplication this spec exists to close, just relocated. The existing ~300-test integration suite (`tests/test_task_kind_unification.rs`) is the regression safety net for this refactor; it must stay green throughout.

**Explicitly out of scope, with rationale** (each independently investigated, not just asserted):

- **`claim_task`/`claim_mesh_task`** — investigated via a dedicated Fable-model pass (see SDD ledger). Structurally inverted from the five in scope: it *generates* a lease rather than verifying an existing one, uses a `version_counter` CAS instead of ownership+freshness, and its real callers (`edgeplaned`'s task loop deserializes the full REST response row and hard-errors otherwise; `solo_supervisor` string-matches 423/409 status codes) pin exact behavior a refactor would endanger. Zero predicate text is shared between claim and the five today, and it has no fencing hole of its own — its real defects (broadcast branch has zero predicate beyond `WHERE id=$1`, MCP ignores `claim_policy` entirely, TTL divergence 120s vs 60-3600s, an unchecked `i32` version overflow) are a different, already-tracked cluster in the plan's roadmap. Gets its own dedicated pass later.
- **`resolve_gate`** — a bespoke transaction (fences a `reviewgate` row by gate ownership, then conditionally transitions the task in the same transaction) with no MCP equivalent at all. Cross-table aggregate and transaction shape are materially different from the five; forcing it into the same primitive would blur, not clarify, the security-relevant distinctions.
- **`cancel_task`/`unblock_task`** — no MCP equivalent exists (`mcp.rs`'s full mesh-task tool list is: submit, list, claim, heartbeat, progress, complete, fail, block — nothing else). REST-side consolidation of these two (shared actor-derivation, shared response helper) is a reasonable low-risk follow-up but isn't required to close the security gap this spec targets.
- **`create_gate`, `retry_task`, `dispatch_task`, `tasks.rs::update_task`** — real, independently-verified gaps (unfenced or check-then-act), but none were ever part of EP-1's endpoint scope and none share the fence shape this primitive formalizes. Logged in the plan's roadmap.

## Architecture

**Not a macro, not one fully-generic function.** The consolidation review that informed this design is explicit on this point, and it matches this whole plan's own hard-won lesson: a predicate hidden behind a macro is exactly the kind of thing that gets copy-pasted wrong without a reviewer noticing, which is how the CRITICAL bug happened in the first place. The design instead is a small, closed **enum-dispatched service** with the actual SQL text visible per "fence family" — auditable, not generic.

### Core types

```rust
pub struct TransitionActor<'a> {
    pub subject: &'a str,       // raw principal.subject, e.g. "agent:xyz"
    pub subject_id: &'a str,    // strip_prefix("agent:") applied
    pub is_bypass: bool,        // is_full_trust(principal) || principal.is_admin
    pub is_admin: bool,
}

pub enum TaskTransition<'a> {
    Heartbeat { claim_lease_id: Option<&'a str> },
    AppendProgress { claim_lease_id: &'a str, event: ProgressInput<'a> },
    Complete { claim_lease_id: Option<&'a str>, agent_id: Option<&'a str>, result_artifact_id: Option<i32> },
    Fail { claim_lease_id: Option<&'a str>, agent_id: Option<&'a str>, error: Option<&'a str> },
    Block,
}

pub enum TransitionOutcome {
    Task { task: TaskRecord, unblocked_task_ids: Vec<String> },
    Progress(ProgressRecord),
    WaitingReview { task: TaskRecord, pending_gate_ids: Vec<String> },
}

#[derive(Debug, thiserror::Error)]
pub enum TransitionError {
    #[error("task not found")]
    NotFound,
    #[error("not authorized for this task")]
    Forbidden,
    #[error("task is not in the required state")]
    Conflict,
    #[error("{0}")]
    Invalid(String),
    #[error("database error during {operation}")]
    Database { operation: &'static str, #[source] source: sqlx::Error },
}

pub async fn execute_task_transition(
    db: &sqlx::PgPool,
    actor: &TransitionActor<'_>,
    task_id: &str,
    transition: TaskTransition<'_>,
) -> Result<TransitionOutcome, TransitionError>;
```

The service owns: task-domain lookup + domain authorization, the fence predicate, the single mutating SQL statement, rejected-write classification (today's `classify_fenced_rejection`, adapted to return `TransitionError` instead of an `axum::Response`), and side-effect facts (pending gate IDs, unblocked-dependent task IDs) needed for REST and MCP to behave consistently. It does **not** own HTTP status codes, MCP's `{ok, result, error}` envelope, or MCP argument parsing from arbitrary `Value` — those stay at the transport edge.

### Fence families (visible, not generic)

Four distinct predicate shapes, each its own small builder/method — not one parameterized function that could silently drift into producing the wrong shape:

- **A — live claimable lease fence** (heartbeat, progress): `kind='claimable' AND status IN ('claimed','running') AND (claim_lease_id = $lease OR $bypass) AND (claim_policy='broadcast' OR lease_expires_at >= $now)`. **Must use a row-locked `UPDATE...WHERE`-shaped statement** (heartbeat's existing shape), not a lock-free CTE — `append_progress`'s current CTE-without-`FOR UPDATE` has an independently-confirmed cross-table TOCTOU (rust-reviewer's live 3-connection reproduction, and a second independent Codex review reaching the same conclusion via SQL-semantics reasoning) that this primitive closes structurally by construction, not by patching the old code.
- **B — lifecycle fence, claimable-or-assigned** (complete, fail): dual ownership path (`claimed_by_agent_id = $effective_id` for claimable, `owner = $effective_id` for assigned) alongside the lease/bypass path, `waiting_review` as a legal claimable source state, on-behalf-of `effective_id` derivation for the full-trust `task_worker.rs` caller (Ruling C2 — this identity-only path is a deliberate, already-reasoned exception to the freshness requirement, not an oversight; removing it breaks a real deployed caller that never heartbeats).
- **D — claimable pause/resume fence** (block): `kind='claimable'`, ownership-by-subject-or-bypass, no lease requirement.

`complete`'s pending-gate CTE, artifact handling, and dependent-unblocking stay explicit inside its own transition arm — not hidden behind the generic service.

### REST and MCP as thin adapters

```rust
// REST
async fn complete_task(...) -> Response {
    rest_result(execute_task_transition(&db, &actor, &id, TaskTransition::Complete{..}).await)
}

// MCP
"complete_mesh_task" => match execute_task_transition(&db, &actor, &id, input.into()).await {
    Ok(outcome) => ok_result(serialize_transition(outcome)),
    Err(e) => mcp_transition_error(e),
}
```

This directly honors Merlin's constraint: MCP's mesh-task handlers become pure translation — parse typed args from `Value`, call the shared service, format the result — with zero independent business logic to drift out of sync. There's already a working precedent for this exact outer shape in this same codebase: `routes/runs.rs`'s four run endpoints all delegate to one `transition()` helper (`runs.rs:143-167`, implementation at `316`).

### CLI-first, minimal MCP footprint

`edgeplane task mesh <verb>` already exists and is a genuine, fully-wired CLI wrapper calling `POST /mcp/call` under the hood (confirmed by reading `crates/edgeplane/src/commands.rs::handle_mesh_task`) — this spec does not need to build new CLI surface; fixing the shared service fixes the CLI path for free, since CLI → MCP tool → shared service → Postgres.

Two things this spec deliberately does *not* attempt, flagged rather than silently dropped:
- **MCP tool-discovery footprint.** `mcp.rs::list_tools()` statically declares all 22 tools in one response — every MCP client gets all 22 schemas upfront, with no JIT/deferred-loading mechanism (unlike this session's own harness). Restructuring that is a materially bigger, separate architectural project than fencing five transitions and is out of this spec's scope. What this spec does honor: it adds zero new MCP tools, and keeps the five relevant handlers' *logic* footprint minimal (arg-parse + one shared-service call + format).
- **Steering agents toward the CLI over raw MCP tool calls** is a documentation/convention question (which path do we tell agents to prefer), not something this spec's code changes decide by themselves.

## Migration approach

1. Implement `execute_task_transition` + the three fence-family builders + `TransitionError`/`TransitionOutcome` in a new module (`crates/edgeplane-tower/src/task_transitions.rs`, exact name TBD in the implementation plan).
2. Refactor REST's five handlers (`heartbeat_task`, `complete_task`, `fail_task`, `block_task`, `append_progress`) to call it, one at a time, full test suite green after each — this is the regression-risk-bearing part, so it goes first and gets the most scrutiny (the existing ~300 tests must not just pass but be re-read to confirm they're still testing the same properties, not just green by coincidence).
3. Wire MCP's five arms (`heartbeat_mesh_task`, `complete_mesh_task`, `fail_mesh_task`, `block_mesh_task`, `progress_mesh_task`) to the same service — this is what actually closes the HIGH-severity lease-revival gap.
4. New MCP-side tests proving the fence (mirroring REST's existing broadcast/stale-lease/idempotent-retry coverage) — MCP currently has none of this.
5. Independent adversarial review (rust-reviewer + security-reviewer, matching every prior task's discipline on this branch) before considering this done.

## Testing

- Every existing REST test in `test_task_kind_unification.rs` must pass unchanged after the handler refactor (behavioral equivalence, not just "compiles").
- New MCP-side fencing tests for all five transitions: the three-test-minimum categories already established for REST (stale-actor retry, concurrent conflicting operation, idempotent retry), plus the broadcast-ownership regression tests REST already has (37dca61a's pattern) — MCP has zero broadcast coverage today, which is exactly how the CRITICAL bug's shape went undetected once already.
- A dedicated test proving the lease-revival exploit chain the security review reproduced live (MCP heartbeat on an expired lease → REST access restored) is now closed.

## Open questions for spec review

- Exact module placement and whether `TaskRecord`/`ProgressRecord` are lightweight structs or JSON-ready DTOs (implementation-plan-level detail, not a design blocker).
- Whether `cancel_task`/`unblock_task` should get the lighter REST-only consolidation (shared actor + response helper, no MCP counterpart) as part of this same plan or as an explicit follow-up — leaning follow-up, to keep this plan's blast radius to the five endpoints with an actual security gap to close.

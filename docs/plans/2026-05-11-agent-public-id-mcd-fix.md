# Agent public_id + mcd Delivery Fix

**Date:** 2026-05-11  
**Author:** aria-merlinlabs  
**Priority:** High — blocks all mcd message delivery to Claude Code agents

---

## Problem

Two bugs prevent mcd from delivering messages to running Claude Code agents:

1. **`/work/` prefix hardcoded in mcd** — all API calls hit `/work/agents/...`, `/work/klusters/...` etc., but mc-controlplane serves these routes without any prefix → connection closed
2. **Agent ID type mismatch** — mcd uses internal UUIDs for agent identity; mc-controlplane uses `i32` integer IDs → `"Cannot parse UUID to i32"`
3. **Integer IDs are unreadable at scale** — `aria-operator id:2`, `aria-work id:7`... by agent 50 the dashboard is noise

---

## Solution

### Part 1 — mc-controlplane: Add `public_id`

**1.1 Migration** — `crates/mc-controlplane/migrations/0008_agent_public_id.sql`

```sql
ALTER TABLE agent ADD COLUMN public_id VARCHAR;
UPDATE agent SET public_id = name || '-' || substr(md5(id::text || random()::text), 1, 8);
ALTER TABLE agent ALTER COLUMN public_id SET NOT NULL;
CREATE UNIQUE INDEX ix_agent_public_id ON agent(public_id);
```

Format: `{agent-name}-{8-char-hash}` → e.g. `aria-work-qwn5eb33`

- Immutable after creation (name changes don't affect it)
- Delete + re-register with same name → new suffix (no stale reference collisions)

**1.2 Model** — `src/models/agent.rs`

Add `pub public_id: String` to `Agent` struct. Include in all response serializations.

**1.3 Route handler** — `src/routes/agents.rs`

- `row_to_agent()` (lines 37-47): map `public_id` from row
- `create_agent()` (lines 105-144): generate on insert via `format!("{}-{}", name, &uuid::Uuid::new_v4().to_string()[..8])`
- Add `AgentIdent` extractor — accepts either `i32` integer or `public_id` string in path params
  - Both `/agents/7` and `/agents/aria-work-qwn5eb33` resolve to the same agent
  - Enables backward compatibility while transitioning
- Apply `AgentIdent` to message/inbox routes (lines 457-493): change `Path<i32>` → `Path<AgentIdent>`

**1.4 Response payloads**

Include `public_id` in all agent responses so mcd can record it after enrollment.

---

### Part 2 — mcd: Strip `/work/` + use public_id

**2.1 Strip the `/work/` prefix — single-place fix**

`crates/mcd/crates/mcd-core/src/client.rs`

Add `api_prefix: String` field (default `""`) to the HTTP client. All hardcoded `/work/` strings become `format!("{}/agents/...", self.api_prefix)` etc.

Affected files (do NOT fix individually — fix at the client layer):
- `mcd/src/task_loop.rs:405` — `/work/agents/{id}/messages`
- `mcd-work/src/task.rs:43,66` — `/work/klusters/...`, `/work/tasks/...`
- `mcd-core/src/client.rs:101` — `/work/missions/{id}/roster`

**2.2 Use public_id from enrollment**

`mcd/src/daemon.rs` — `agent_spec_from_json()` (lines 827-881):

After controlplane returns enrolled agent JSON, read `public_id` field and store as `AgentSpec.agent_id` instead of internal UUID.

`AgentHandle.agent_id` is already `String` — no type change needed.

**2.3 Local SQLite registry**

`mcd/src/local_registry.rs` (line 59): update upsert to store `public_id` from controlplane response, not the local UUID.

---

### Part 3 — mc CLI: Display + routing

**3.1** — `mc daemon agent ls`: show `public_id` as primary identifier, drop numeric `id` from default output  
**3.2** — `mc agent remote message --agent-id` / `--to-agent-id`: already accepts strings; works once route handler accepts public_id  
**3.3** — MC dashboard/TUI: replace numeric `id` column with `public_id`

---

## Implementation Sequence

```
Step 1: mc-controlplane — migration + public_id generation (foundation)
Step 2: mc-controlplane — AgentIdent extractor + route updates
Step 3: mcd — client prefix fix (unblocks ALL /work/ routes)
Step 4: mcd — enrollment reads public_id, stores it, uses for polls
Step 5: mc CLI + dashboard — display update (can trail, cosmetic)
```

Steps 1-2 and Steps 3-4 can be parallelized on separate branches.
Steps 3-4 depend on Step 1 being deployed (need public_id in the response).

**Suggested branch:** `feat/agent-public-id` covering Steps 1-4 together — they need to ship atomically since mcd enrollment must read a public_id that the controlplane generates.

---

## What This Unblocks

End-to-end message delivery:
```
mc agent remote message --agent-id aria-merlinlabs-x1y2z3a4 --to-agent-id aria-work-qwn5eb33 --content "wake up"
  → controlplane resolves public_id → stores in inbox
  → mcd polls /agents/aria-work-qwn5eb33/messages (no /work/ prefix, correct ID)
  → mcd injects into Claude Code session via ACP
  → aria-work acts on the message
```

---

## Key Files

| File | Change |
|------|--------|
| `crates/mc-controlplane/migrations/0008_agent_public_id.sql` | New migration |
| `crates/mc-controlplane/src/models/agent.rs` | Add public_id field |
| `crates/mc-controlplane/src/routes/agents.rs` | AgentIdent extractor, generate on insert |
| `crates/mcd/crates/mcd-core/src/client.rs` | Strip api_prefix |
| `crates/mcd/crates/mcd/src/daemon.rs` | Read public_id from enrollment |
| `crates/mcd/crates/mcd/src/local_registry.rs` | Store public_id |

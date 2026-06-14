# edgeplane — Agent Identity & Lifecycle

**Date:** 2026-05-11
**Status:** Design — pending review
**Owner:** mc-engineer
**Context:** Today `GET /agents` returns one row per `register_agent` call, regardless of whether the same logical agent has registered before. Re-registrations under slightly different names create permanent ghost rows. The TUI surfaces this as duplicates and offline corpses with no way to manage them. The data model conflates "identity" with "session."

This spec separates the two and defines the UX for managing the result.

---

## Today's data quality (audit)

Snapshot from `GET /agents` on 2026-05-11:

```
id 2  aria-operator       online   2026-05-11T11:24:22   claude-code,orchestration,fleet-management
id 3  merlinlabs          offline  2026-05-11T11:42:01   claude-code,k8s,infra,homelab
id 4  aria-mc-engineer    online   2026-05-11T11:50:05   fleet-management,edgeplane-development,...
id 5  anonymous           offline  2026-05-11T11:50:05   unknown
id 6  aria-merlinlabs     online   2026-05-11T12:19:50   claude-code,k8s,infra,homelab
```

Three failure modes visible in five rows:

1. **Re-registration under a new name creates a ghost.** `merlinlabs` (id 3) is the same logical agent as `aria-merlinlabs` (id 6) — same node, same capabilities, registered ~40 min apart. The first row will live forever in `offline` state.
2. **Spurious `anonymous` rows.** id 5 was created at `11:50:05.367` — 50ms after id 4's registration call. Some bootstrap path is auto-creating an `anonymous` row as a side effect.
3. **No GC for stale offline agents.** Nothing reaps rows that haven't heartbeat in days/weeks.

These bugs compound: the longer the controlplane runs, the more landfill accumulates, and the TUI agents screen becomes progressively less useful.

---

## Goals

1. **One row per agent identity.** Re-registration under the same name is an upsert, not an insert.
2. **Session history as a first-class concept.** Every process lifetime / connection is recorded against the agent identity, not folded into it.
3. **Stats roll up.** Total uptime, session count, tokens consumed, last seen, model — all visible per agent.
4. **Lifecycle is explicit and manageable.** Archive, hard-delete, rename — all available from the TUI, none requiring DB surgery.
5. **No spurious rows.** `anonymous` is a reserved name, not a side-effect artifact.

---

## Data model

### `agents` table (existing — repurposed)

One row per **stable identity**. The natural key is `(name, controlplane_id)` where `controlplane_id` is the host this agent belongs to (relevant in multi-controlplane federation).

```sql
-- existing columns, with migration:
id           BIGSERIAL PRIMARY KEY        -- internal, opaque
name         TEXT NOT NULL                -- the stable identity (e.g. "aria-operator")
capabilities TEXT                         -- latest declared capabilities
status       TEXT                         -- derived: 'online' | 'offline' | 'archived'
metadata     JSONB                        -- declared by the agent on register
created_at   TIMESTAMPTZ                  -- first time this name was seen
updated_at   TIMESTAMPTZ                  -- last heartbeat or status change

-- new columns:
archived_at  TIMESTAMPTZ NULL             -- soft-delete marker; archived rows hidden by default
display_name TEXT NULL                    -- user-editable label, falls back to name
node_id      TEXT NULL                    -- node this agent lives on (federation-aware)
last_seen_at TIMESTAMPTZ                  -- last heartbeat — distinct from updated_at

UNIQUE (name, controlplane_id)            -- enforced; upsert semantics
```

`register_agent(name, capabilities)` becomes an `INSERT … ON CONFLICT (name, controlplane_id) DO UPDATE`: capabilities refreshed, `last_seen_at` updated, `archived_at` cleared (re-registering un-archives).

### `agent_sessions` table (new)

One row per process lifetime / WS connection.

```sql
CREATE TABLE agent_sessions (
  id              BIGSERIAL PRIMARY KEY,
  agent_id        BIGINT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
  started_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  ended_at        TIMESTAMPTZ NULL,
  node_id         TEXT NULL,              -- where this session ran
  pid             INTEGER NULL,           -- process ID at session start
  model           TEXT NULL,              -- e.g. claude-sonnet-4-6
  runtime_kind    TEXT NULL,              -- e.g. claude_code, claude_code_acp, goose
  session_kind    TEXT NOT NULL,          -- 'task' | 'persistent'
  prompts_count   INTEGER DEFAULT 0,      -- updated by progress events
  tokens_input    BIGINT DEFAULT 0,       -- rolled up from per-turn telemetry
  tokens_output   BIGINT DEFAULT 0,       -- rolled up from per-turn telemetry
  exit_reason     TEXT NULL,              -- 'clean' | 'crash' | 'killed' | 'restart' | NULL while live
  metadata        JSONB DEFAULT '{}'::jsonb
);

CREATE INDEX agent_sessions_agent_idx ON agent_sessions(agent_id, started_at DESC);
CREATE INDEX agent_sessions_live_idx ON agent_sessions(ended_at) WHERE ended_at IS NULL;
```

A live session is one with `ended_at IS NULL`. There can be **at most one live session per agent** — invariant enforced via partial unique index on `(agent_id) WHERE ended_at IS NULL`.

### Status derivation

The `agents.status` field becomes a derived rule, not a stored truth (or stored but always recomputed):

| Condition | Status |
|-----------|--------|
| `archived_at IS NOT NULL` | `archived` |
| Live session exists AND `last_seen_at` within 60s | `online` |
| Live session exists AND `last_seen_at` 60s–5min ago | `stale` |
| No live session OR `last_seen_at` >5min ago | `offline` |

The 5-minute threshold for marking a session "ended without notice" is a periodic reaper job: it closes sessions whose agent hasn't heartbeat in 5+ minutes, setting `exit_reason = 'lost-heartbeat'`.

---

## API changes

### Existing endpoints

- `POST /agents` (register) — upsert by `(name, controlplane_id)`. Creates a new row in `agent_sessions` and returns the session_id alongside agent_id. Re-registering an archived agent un-archives it. If a live session already exists, the old one is closed (`exit_reason = 'superseded'`).
- `GET /agents` — by default, returns only non-archived rows. Query param `?include_archived=true` returns everything.
- `PATCH /agents/{id}/status` — accepts `online` | `offline` only. Updates the live session's `last_seen_at`; if status flips to `offline`, closes the live session.

### New endpoints

- `GET /agents/{id}/sessions?limit=20&since=...` — paginated session history.
- `GET /agents/{id}/stats?window=30d` — rolled-up stats over the window.
- `POST /agents/{id}/archive` — soft-delete. Closes the live session (if any), sets `archived_at`.
- `POST /agents/{id}/unarchive` — clears `archived_at`. Does not create a new session.
- `DELETE /agents/{id}` — hard-delete. Allowed only when `archived_at IS NOT NULL` AND no sessions in the last 30 days. Cascades to `agent_sessions`.
- `PATCH /agents/{id}` — rename (`{ "display_name": "..." }`), or update declared metadata. The immutable `name` (stable key) cannot be changed via API.

### `anonymous` is reserved

`name = 'anonymous'` and any name starting with `system:` are reserved. Registration with these names returns `409 Conflict`. The bug that creates spurious anonymous rows gets fixed at the same time — but the name reservation is the belt+suspenders so it doesn't come back.

---

## TUI surface

### List layout — three buckets

```
┌─ Agents ──────────────────────────────────────────────────────────┐
│                                                                   │
│  ● Connected (3)                                                  │
│  ▶ aria-operator         excalibur · 4d 6h up      ⚡ active      │
│    aria-mc-engineer      excalibur · 12m up        ⚡ idle        │
│    aria-merlinlabs       excalibur · 2h up         ⚡ idle        │
│                                                                   │
│  ○ Recent (1)                                                     │
│    aria-research         last seen 14m ago         ○ offline      │
│                                                                   │
│  ▾ Archived (4)                                              [a]  │
│                                                                   │
└───────────────────────────────────────────────────────────────────┘
```

- **Connected** — `online` or `stale`, expanded by default. Sorted by uptime descending.
- **Recent** — `offline`, `last_seen_at` within last 7 days. Expanded. Sorted by `last_seen_at` desc.
- **Archived** — collapsed by default; `a` toggles visibility. Sorted by `archived_at` desc.

Status pill on each row: ⚡ active (live + heartbeat within 10s), ⚡ idle (live + no recent activity), ○ offline, ⌀ archived.

### Detail pane

Right side or modal — depending on terminal width — for the cursored agent:

```
┌─ aria-operator ───────────────────────────────────────────────────┐
│  Status         online · idle                                     │
│  Node           excalibur                                         │
│  Model          claude-sonnet-4-6                                 │
│  Runtime        claude_code_acp (persistent)                      │
│  Capabilities   claude-code, orchestration, fleet-management      │
│                                                                   │
│  Current session   4d 6h up · 1,247k tokens · 89 prompts          │
│  Last 30 days      31d cumulative uptime · 5 sessions             │
│                    4.2M tokens (3.1M in · 1.1M out)               │
│                    3 clean exits · 2 lost-heartbeat               │
│  First seen        2026-04-18                                     │
│  Last restart      2026-05-07 (clean)                             │
│                                                                   │
│  Session history   [press s to view]                              │
│                                                                   │
│  Actions:                                                         │
│  [ Attach ]  [ Restart ]  [ Clear context ]  [ Signal… ]          │
│  [ Rename ]  [ Archive ]                                          │
│                                                                   │
└───────────────────────────────────────────────────────────────────┘
```

Action affordances:
- **Attach** — opens the ACP message stream (Phase 2 of persistent-session work). Disabled if no live session.
- **Restart** — calls `POST /agents/{id}/restart`. Closes the current session cleanly and signals edgeplaned to relaunch.
- **Clear context** — calls `POST /agents/{id}/clear-context`. Disabled if no live session.
- **Signal** — opens an input modal to send a prompt/signal to the live session.
- **Rename** — updates `display_name`. The stable `name` (used for upsert) cannot be changed here.
- **Archive** — soft-delete. Confirmation modal: "Archive aria-operator? This stops the live session and moves it to Archived. You can restore at any time."

### Session history view (`s`)

```
┌─ aria-operator · sessions ─────────────────────────────────────────┐
│                                                                    │
│  ● 2026-05-07 → now       4d 6h    1.2M tok    persistent          │
│    2026-05-03 → 2026-05-07 4d      980k tok    persistent  clean   │
│    2026-04-29 → 2026-05-03 4d      1.1M tok    persistent  crash   │
│    2026-04-22 → 2026-04-29 7d      640k tok    persistent  clean   │
│    2026-04-18 → 2026-04-22 4d      210k tok    persistent  killed  │
│                                                                    │
│  [Enter] inspect session   [Esc] back                              │
│                                                                    │
└────────────────────────────────────────────────────────────────────┘
```

Tab into a session row → drill-down view with that session's progress events, exit reason, and (for the live one) the live attach.

### Archived bucket

Same layout, but with two extra actions on the detail pane:

- **Unarchive** — restores. The agent will re-appear in Recent until a new session starts.
- **Delete permanently** — two-step destructive confirmation. Only enabled if no sessions in the last 30 days.

---

## Migration

Existing rows in `agents`:

1. **Add new columns** — `archived_at`, `display_name`, `node_id`, `last_seen_at` (nullable). Backfill `last_seen_at` from `updated_at`.
2. **Detect identity duplicates by node + capabilities** — flag rows likely to be the same agent (current snapshot would flag `merlinlabs` + `aria-merlinlabs`). Print a one-time migration report to the operator. Do **not** auto-merge — operator picks the canonical name via `edgeplane admin agents merge --keep <id> --drop <id>`.
3. **Reap `anonymous` rows** automatically during the migration. Their `name` becomes a reserved value going forward.
4. **Create `agent_sessions` table** and seed one row per existing agent that's currently `online`, with `started_at = updated_at` (best approximation).
5. **Enforce the unique index** `(name, controlplane_id)` after deduplication.

The migration is reversible up to the `DELETE`s of `anonymous` rows. Migration runs in a single transaction; pre-flight dry-run mode shows what would change.

---

## Implementation plan

### Phase 1 — Schema + upsert semantics

- Migration `0006_agent_identity.sql` (additive, no breaking changes yet).
- `register_agent` route uses `INSERT … ON CONFLICT … DO UPDATE`.
- Reserved-name guard (`anonymous`, `system:*`).
- Reap spurious `anonymous` rows.
- `GET /agents` filters archived by default.

**Result:** No new duplicates. Existing ones remain visible until manually merged.

### Phase 2 — Sessions table

- Migration `0007_agent_sessions.sql`.
- `register_agent` creates `agent_sessions` row.
- `update_agent_status` closes session on `offline`.
- Heartbeat reaper (5min threshold).
- Stats endpoints: `/agents/{id}/sessions`, `/agents/{id}/stats`.

**Result:** Session history available. Stats roll up. No TUI changes yet.

### Phase 3 — Lifecycle endpoints + admin merge tool

- `archive`, `unarchive`, `DELETE`, `PATCH` (rename) endpoints.
- `edgeplane admin agents merge` CLI command for the migration cleanup.
- `edgeplane admin agents reap --older-than 30d` for periodic cleanup.

**Result:** Operators can clean up landfill from previous duplicates.

### Phase 4 — TUI screen rework

- Three-bucket list (Connected / Recent / Archived).
- Detail pane with stats + actions.
- Session history view.
- Confirmation modals for destructive actions.

**Result:** The agents screen becomes the front door to fleet management.

Phase 1 alone fixes the new-duplicate problem. Phases 2-4 build the value.

---

## Open questions

1. **Token accounting source.** The detail pane shows per-session tokens. Where do those numbers come from today? If the runtime doesn't already emit `tokens_input` / `tokens_output` per turn, we need to wire that — and it's coupled to whether we're using ACP (structured events with usage data) or raw PTY (no per-turn telemetry). The ACP-first decision (`project_mc_acp_first.md`) makes this cleaner.

2. **Heartbeat cadence.** 60s "online" threshold assumes a heartbeat at least that frequently. Today's cadence in edgeplaned? Worth verifying before pinning the threshold.

3. **Federation: same agent name on two controlplanes.** Today the `(name, controlplane_id)` uniqueness handles this. But if a node moves between controlplanes, do we want a single global identity? **Proposed:** no — each controlplane owns its agents. Federation can show a federated view but the rows are distinct.

4. **`edgeplane admin agents merge` UX.** Should the merge be interactive (TUI flow that walks through candidates) or strictly CLI? **Proposed:** CLI first (simple), TUI version as a follow-up if operators actually need it.

5. **Live-session uniqueness vs registration races.** If two `edgeplaned` processes register the same agent name simultaneously, the partial unique index will reject the second one. Should it be a friendly "another instance is already running" error or a forced-takeover? **Proposed:** friendly error. Force-takeover is too easy to misuse.

---

## Acceptance criteria

- [ ] Registering `aria-operator` twice produces one row, not two.
- [ ] `aria-operator` running for 30 days shows 30d cumulative uptime even across restarts.
- [ ] An agent that crashes and reconnects shows up as one row with two sessions.
- [ ] An agent absent for >5min is marked offline automatically by the reaper.
- [ ] An archived agent disappears from the default `GET /agents` response.
- [ ] An archived agent can be unarchived and shows up again with no data loss.
- [ ] Hard-deleting an agent younger than 30 days is rejected.
- [ ] No code path can create a row named `anonymous`.
- [ ] Running the migration against the current snapshot produces a clean state without operator surgery beyond the documented `merge` command.

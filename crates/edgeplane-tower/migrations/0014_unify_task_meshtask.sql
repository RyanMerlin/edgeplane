-- PR1 of the task/meshtask merge: legacy `task` (integer PK, human/PM-facing --
-- owner, epic, definition_of_done, dashboard-consumed) and `meshtask` (varchar
-- PK, agent-claimable -- capability routing, claim/lease, dispatched via
-- CLI/MCP/daemon) are two disconnected "unit of work" tables with zero
-- synchronization (already produced a real shipped bug: PR #108's workspace-
-- snapshot handler read `task` while every real claim/complete wrote to
-- `meshtask`). This migration makes `meshtask` the single surviving
-- primitive: extend it with the columns needed to also carry `task`'s
-- human-facing fields plus a `kind` discriminator ('assigned' | 'claimable'),
-- migrate every legacy `task` row in as kind='assigned', retype+remap the
-- satellite tables that pointed at the old integer id, drop the legacy table
-- and its write-dead siblings, then rename meshtask -> task so the surviving
-- primitive owns the short name. Existing FKs onto meshtask(id)
-- (agentrun.mesh_task_id, reviewgate.mesh_task_id, usagerecord.mesh_task_id)
-- follow the rename automatically -- no redefinition needed.
--
-- Rust callers are intentionally NOT updated in this migration (follow-up
-- PR); the crate will not compile against this schema until that lands.

-- ── 1. Extend meshtask with the assigned/claimable-unified columns ──────────
--
-- kind defaults to 'claimable' so every pre-existing meshtask row (all of
-- which are today's agent-claimable rows) backfills correctly the instant
-- the column is added; the legacy-task migration in step 4 overrides it to
-- 'assigned' per row. The default is dropped right after so future inserts
-- must state kind explicitly, matching how the app always sets columns it
-- cares about.
ALTER TABLE public.meshtask
    ADD COLUMN IF NOT EXISTS public_id character varying,
    ADD COLUMN IF NOT EXISTS kind character varying NOT NULL DEFAULT 'claimable'
        CONSTRAINT ck_meshtask_kind CHECK (kind IN ('assigned', 'claimable')),
    ADD COLUMN IF NOT EXISTS owner character varying,
    ADD COLUMN IF NOT EXISTS contributors character varying,
    ADD COLUMN IF NOT EXISTS done_criteria text,
    ADD COLUMN IF NOT EXISTS dependencies_note text,
    ADD COLUMN IF NOT EXISTS related_artifacts_note text,
    ADD COLUMN IF NOT EXISTS attempt smallint NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS max_attempts smallint NOT NULL DEFAULT 1,
    ADD COLUMN IF NOT EXISTS attempted_by text[],
    ADD COLUMN IF NOT EXISTS errors jsonb[],
    ADD COLUMN IF NOT EXISTS finalized_at timestamptz;

ALTER TABLE public.meshtask ALTER COLUMN kind DROP DEFAULT;

-- claim_policy was NOT NULL under the meshtask-only regime (every row was
-- claimable). Under the unified schema it's claimable-only and must be able
-- to hold NULL for kind='assigned' rows, so relax it. Not spelled out in the
-- column list this migration was scoped from -- see report for why it's
-- needed anyway.
ALTER TABLE public.meshtask ALTER COLUMN claim_policy DROP NOT NULL;

-- ── 2. Retype result_artifact_id (varchar) -> integer to match artifact.id ──
--
-- artifact.id is integer (crates/edgeplane-tower/migrations/
-- 0001_initial_schema.sql: `CREATE TABLE public.artifact (id integer NOT
-- NULL, ...)`). No code path writes meshtask.result_artifact_id today, so
-- this UPDATE is expected to be a no-op in every real environment; it's a
-- defensive guard so the type change can't fail the migration if some
-- out-of-band value doesn't cast cleanly to integer -- such values are
-- nulled rather than aborting the migration.
UPDATE public.meshtask
   SET result_artifact_id = NULL
 WHERE result_artifact_id IS NOT NULL
   AND result_artifact_id !~ '^[0-9]+$';

ALTER TABLE public.meshtask
    ALTER COLUMN result_artifact_id TYPE integer USING result_artifact_id::integer;

-- ── 3. Id-mapping table: legacy task.id (integer) -> new meshtask.id (varchar) ──
--
-- A real table (not TEMP) so it survives independent of connection/session
-- boundaries within this migration; dropped in step 7 once every satellite
-- remap below has consumed it.
CREATE TABLE public.task_id_migration_map (
    old_int_id    integer NOT NULL PRIMARY KEY,
    new_string_id character varying NOT NULL UNIQUE
);

INSERT INTO public.task_id_migration_map (old_int_id, new_string_id)
SELECT id, gen_random_uuid()::text
  FROM public.task;

-- ── 4. Migrate legacy task rows into meshtask as kind='assigned' ────────────
--
-- Field mapping: owner/contributors/title/description/status/created_at/
-- updated_at copied directly; dependencies -> dependencies_note and
-- related_artifacts -> related_artifacts_note (both were free-text CSV, now
-- display-only notes); definition_of_done -> done_criteria. public_id is
-- generated fresh ({prefix}-{8 hex chars}, matching routes/agents.rs::
-- generate_public_id's convention) since legacy task.public_id was always
-- written as '' (see routes/tasks.rs create_task: `VALUES ('',$1,...)`) --
-- not a real second identity worth preserving.
--
-- domain_id is backfilled by joining through mission.domain_id via a plain
-- (inner) join: every application-created mission always sets domain_id at
-- creation (routes/missions.rs::create_mission binds it from the route's
-- domain_id path param), even though the column itself is nullable at the
-- DB level. A legacy task whose mission_id doesn't resolve to a mission row,
-- or whose mission has a NULL domain_id, is intentionally left unmigrated
-- (and therefore lost when the legacy `task` table is dropped in step 7)
-- rather than guessing a domain -- flagged prominently in the report as a
-- real pre-flight check for whoever eventually runs this against live data.
--
-- created_by_subject has no better source than owner on the legacy schema
-- (task never tracked a separate creator identity) -- ASSUMPTION.
INSERT INTO public.meshtask (
    id, public_id, mission_id, domain_id, kind,
    title, description, status, owner, contributors,
    done_criteria, dependencies_note, related_artifacts_note,
    priority, version_counter, created_by_subject, created_at, updated_at
)
SELECT
    map.new_string_id,
    'task-' || substr(replace(gen_random_uuid()::text, '-', ''), 1, 8),
    t.mission_id,
    mi.domain_id,
    'assigned',
    t.title,
    t.description,
    t.status,
    t.owner,
    t.contributors,
    t.definition_of_done,
    t.dependencies,
    t.related_artifacts,
    0,
    0,
    t.owner,
    t.created_at,
    t.updated_at
  FROM public.task t
  JOIN public.mission mi ON mi.id = t.mission_id
  JOIN public.task_id_migration_map map ON map.old_int_id = t.id;

-- ── 5. Backfill public_id for pre-existing meshtask rows, then constrain ────
--
-- Rows inserted in step 4 already have public_id set; this only touches
-- rows that existed before this migration (today's claimable meshtask rows,
-- which never had a public_id column at all).
UPDATE public.meshtask
   SET public_id = 'task-' || substr(replace(gen_random_uuid()::text, '-', ''), 1, 8)
 WHERE public_id IS NULL;

ALTER TABLE public.meshtask ALTER COLUMN public_id SET NOT NULL;
CREATE UNIQUE INDEX ix_meshtask_public_id ON public.meshtask USING btree (public_id);

-- ── 6. Retype + remap satellite FKs from legacy integer task.id ─────────────
--
-- Add-backfill-drop-rename per column: a straight ALTER COLUMN TYPE varchar
-- USING task_id::varchar would just stringify the old integer (e.g. 42 ->
-- '42'), not remap to the new meshtask id -- the join through
-- task_id_migration_map is required to actually remap values.
ALTER TABLE public.agentmessage ADD COLUMN IF NOT EXISTS task_id_str character varying;
UPDATE public.agentmessage am
   SET task_id_str = map.new_string_id
  FROM public.task_id_migration_map map
 WHERE map.old_int_id = am.task_id;
ALTER TABLE public.agentmessage DROP COLUMN IF EXISTS task_id;
ALTER TABLE public.agentmessage RENAME COLUMN task_id_str TO task_id;

ALTER TABLE public.overlapsuggestion
    ADD COLUMN IF NOT EXISTS task_id_str character varying,
    ADD COLUMN IF NOT EXISTS candidate_task_id_str character varying;
UPDATE public.overlapsuggestion os
   SET task_id_str = map.new_string_id
  FROM public.task_id_migration_map map
 WHERE map.old_int_id = os.task_id;
UPDATE public.overlapsuggestion os
   SET candidate_task_id_str = map.new_string_id
  FROM public.task_id_migration_map map
 WHERE map.old_int_id = os.candidate_task_id;
ALTER TABLE public.overlapsuggestion
    DROP COLUMN IF EXISTS task_id,
    DROP COLUMN IF EXISTS candidate_task_id;
ALTER TABLE public.overlapsuggestion RENAME COLUMN task_id_str TO task_id;
ALTER TABLE public.overlapsuggestion RENAME COLUMN candidate_task_id_str TO candidate_task_id;
-- Both columns were NOT NULL integer originally; preserve that -- any row
-- whose old id didn't resolve through the mapping table (dangling FK-less
-- reference) would surface here as a NOT NULL violation rather than
-- silently persisting a broken remap.
ALTER TABLE public.overlapsuggestion
    ALTER COLUMN task_id SET NOT NULL,
    ALTER COLUMN candidate_task_id SET NOT NULL;

CREATE INDEX ix_overlapsuggestion_task_id ON public.overlapsuggestion USING btree (task_id);
CREATE INDEX ix_overlapsuggestion_candidate_task_id ON public.overlapsuggestion USING btree (candidate_task_id);

-- ── 7. Drop dead/write-dead tables and the migration scratch table ──────────
--
-- taskassignment: no INSERT/UPDATE/SELECT anywhere in the Rust codebase --
-- only a cleanup `DELETE FROM taskassignment WHERE task_id=$1` in
-- routes/tasks.rs's task-delete handler.
-- epic: zero Rust references anywhere; task.epic_id is only ever written
-- back to itself in tasks.rs's own CRUD (insert/update), never joined or
-- read elsewhere. Dropping `task` below removes task.epic_id with it.
-- task_id_seq is `OWNED BY task.id` and is dropped automatically by
-- Postgres when `task` is dropped -- no separate DROP SEQUENCE needed.
DROP TABLE IF EXISTS public.taskassignment;
DROP TABLE IF EXISTS public.epic;
DROP TABLE IF EXISTS public.task;
DROP TABLE IF EXISTS public.task_id_migration_map;

-- ── 8. meshtask becomes the one task primitive ───────────────────────────────
--
-- Existing FKs (agentrun.mesh_task_id, reviewgate.mesh_task_id,
-- usagerecord.mesh_task_id) follow the rename automatically. Existing
-- indexes/constraints also follow the rename but keep their old names
-- (meshtask_pkey, ix_meshtask_status, ck_meshtask_kind, etc.) -- Postgres
-- does not rename them for you. Purely cosmetic; left as a follow-up.
ALTER TABLE public.meshtask RENAME TO task;

-- ── 9. New indexes for the unified access patterns ──────────────────────────
--
-- Fast lookup of a human/agent's assigned work; fast reclaim sweep over
-- claimed rows whose lease has expired (SQS/pgmq visibility-timeout
-- pattern).
CREATE INDEX ix_task_owner_status ON public.task USING btree (owner, status) WHERE kind = 'assigned';
CREATE INDEX ix_task_lease_expires_at ON public.task USING btree (lease_expires_at) WHERE status = 'claimed';

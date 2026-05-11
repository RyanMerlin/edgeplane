-- Phase 1 of agent-identity rework (see docs/plans/mc-agents-identity-spec.md).
--
-- This migration is additive: it adds nullable columns for lifecycle metadata
-- and reaps existing spurious `anonymous` agent rows that were created as a
-- side-effect of unauthenticated hook calls. The schema-level enforcement of
-- the reserved-name list happens in code (routes/hooks.rs and routes/mcp.rs)
-- rather than via a CHECK constraint, so the migration stays reversible.
--
-- Columns are nullable so the migration can run without coordinating with
-- in-flight inserts; callers that don't set them today behave exactly as
-- before. Phases 2-3 of the spec build on these columns.

-- Lifecycle metadata for the agent identity row.
ALTER TABLE public.agent
    ADD COLUMN IF NOT EXISTS archived_at   timestamp without time zone NULL,
    ADD COLUMN IF NOT EXISTS display_name  character varying NULL,
    ADD COLUMN IF NOT EXISTS node_id       character varying NULL,
    ADD COLUMN IF NOT EXISTS last_seen_at  timestamp without time zone NULL;

-- Backfill last_seen_at from updated_at so existing rows have a sensible
-- value the moment Phase 2 starts depending on it. Idempotent: only fills
-- rows where last_seen_at is still NULL.
UPDATE public.agent
   SET last_seen_at = updated_at
 WHERE last_seen_at IS NULL;

-- An index on archived_at keeps the "non-archived" list_agents query cheap
-- once Phase 2 layers on session aggregation.
CREATE INDEX IF NOT EXISTS ix_agent_archived_at
    ON public.agent (archived_at)
    WHERE archived_at IS NOT NULL;

-- Reap existing spurious `anonymous` rows. These were created by hook calls
-- arriving without auth — that path now refuses to create reserved-name
-- agents (see routes/hooks.rs), so this DELETE is a one-shot cleanup.
DELETE FROM public.agent WHERE name = 'anonymous';

-- Add a stable, human-readable public identifier for agents.
--
-- Motivation:
-- - Integer ids (2, 7, 8...) become unreadable noise at fleet scale.
-- - mc-mesh uses string identifiers (UUIDs today); mc-controlplane uses i32.
--   Aligning on a single string identifier removes the type mismatch and
--   the per-route parsing it produces ("Cannot parse UUID to i32").
-- - Format: `{name}-{8-char-suffix}`. The suffix is short enough to stay
--   readable in CLI/TUI output and unique enough across delete/recreate
--   cycles (so a re-registered `aria-work` doesn't collide with the
--   previous one's id).
-- - Immutable after creation. The unique index lives on (public_id), not
--   on the prefix — renames don't change the id.

ALTER TABLE public.agent
    ADD COLUMN IF NOT EXISTS public_id varchar NULL;

-- Backfill existing rows. Uses md5 of (id||random()) for the suffix —
-- chosen over uuid_generate_v4 to avoid requiring the uuid-ossp extension
-- in this migration. Application-generated public_ids (in create_agent)
-- use uuid::Uuid::new_v4 for cleaner randomness, but both produce 8 hex
-- chars in the same shape.
UPDATE public.agent
   SET public_id = name || '-' || substr(md5(id::text || random()::text), 1, 8)
 WHERE public_id IS NULL;

-- Lock the invariant once everyone has one.
ALTER TABLE public.agent
    ALTER COLUMN public_id SET NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS ix_agent_public_id
    ON public.agent (public_id);

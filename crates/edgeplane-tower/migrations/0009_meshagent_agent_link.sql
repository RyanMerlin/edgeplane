-- Link mesh-agent rows to the canonical agent identity row.
--
-- Background: the `meshagent` table holds runtime topology (which agent
-- runtime is enrolled on which node). The `agent` table holds the
-- persistent agent identity (aria-operator, aria-work, ...). They were
-- never linked, so edgeplaned's poll URL — `/agents/{public_id}/messages`,
-- which resolves against `agent.public_id` — couldn't find the inbox for
-- a meshagent identified only by its random UUID. See
-- `docs/plans/2026-05-11-agent-public-id-edgeplaned-fix.md`.
--
-- This migration adds the link. New enrollments populate it via
-- enroll_agent / assign_node_agent / enroll_mesh_agent (MCP) / home-mission
-- provisioning. Existing rows stay NULL; the JSON serializer falls back to
-- `meshagent.id` so pre-link rows keep working.

ALTER TABLE public.meshagent
    ADD COLUMN IF NOT EXISTS agent_public_id varchar NULL;

CREATE INDEX IF NOT EXISTS ix_meshagent_agent_public_id
    ON public.meshagent (agent_public_id)
    WHERE agent_public_id IS NOT NULL;

-- Phase 4e (capability discovery) writes here at agent connect time:
-- ACP-shaped runtimes push their `InitializeResponse.agentCapabilities`
-- (flattened to dotted strings) into this column on every supervised
-- session start. Last-seen wins.
--
-- The capability dispatcher unions this with `meshagent.capabilities`
-- (user-set) and the runtime's built-in capability list when matching
-- against `task.required_capabilities`.
DO $$ BEGIN
    ALTER TABLE public.meshagent ADD COLUMN discovered_capabilities text;
EXCEPTION WHEN duplicate_column THEN NULL; END $$;

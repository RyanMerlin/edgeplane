-- Migration 0006: add mission.kind to distinguish node-level home missions
-- from regular work missions.
--
-- Phase 6 (home-mission + sync-loop) provisions a `home-{tailscale_hostname}`
-- mission for each registered node, hosting a persistent Goose agent that
-- routes work into domain missions. The `kind` column lets the UI and API
-- filter home missions into their own section without inventing a new table.
--
-- Values:
--   'work' — regular mission (default; matches all existing rows)
--   'home' — per-node coordination inbox
--
-- Idempotent: safe to re-run.

ALTER TABLE public.mission
    ADD COLUMN IF NOT EXISTS kind character varying NOT NULL DEFAULT 'work';

CREATE INDEX IF NOT EXISTS ix_mission_kind ON public.mission USING btree (kind);

-- Migration 0012: SWAP rename — Mission → Domain, Kluster → Mission
--
-- Context: the entity vocabulary is being swapped for clarity:
--   old Mission (org boundary)   → Domain
--   old Kluster (workstream)     → Mission
--   Task stays unchanged
--
-- All renames happen in a single transaction. Within each "collision" table
-- (those that have BOTH mission_id and kluster_id) the old mission_id column
-- MUST be renamed to domain_id BEFORE kluster_id is renamed to mission_id,
-- to avoid a transient name collision within the same table.
--
-- The brand "MissionControl" and all binary/env/chart names are unchanged.
-- Only the entity vocabulary changes.

BEGIN;

-- ============================================================
-- 1. TABLE RENAMES — old-mission group
-- ============================================================

-- Rename primary entity tables first so later FK/index ops use new names.
ALTER TABLE public.mission                 RENAME TO domain;
ALTER TABLE public.missionrolemembership   RENAME TO domainrolemembership;
ALTER TABLE public.evolvemission           RENAME TO evolvedomain;
ALTER TABLE public.missionpack             RENAME TO domainpack;
ALTER TABLE public.missionpersistencepolicy  RENAME TO domainpersistencepolicy;
ALTER TABLE public.missionpersistenceroute   RENAME TO domainpersistenceroute;

-- ============================================================
-- 2. TABLE RENAME — old-kluster becomes mission
-- ============================================================

ALTER TABLE public.kluster RENAME TO mission;

-- ============================================================
-- 3. CONSTRAINT RENAMES on renamed tables
-- ============================================================

-- domain (was mission)
ALTER TABLE public.domain RENAME CONSTRAINT mission_pkey TO domain_pkey;
ALTER TABLE public.domain RENAME CONSTRAINT ck_mission_owners_nonempty TO ck_domain_owners_nonempty;

-- mission (was kluster)
ALTER TABLE public.mission RENAME CONSTRAINT kluster_pkey TO mission_pkey;
ALTER TABLE public.mission RENAME CONSTRAINT ck_kluster_owners_nonempty TO ck_mission_owners_nonempty;

-- domainrolemembership (was missionrolemembership)
ALTER TABLE public.domainrolemembership RENAME CONSTRAINT missionrolemembership_pkey TO domainrolemembership_pkey;

-- evolvedomain (was evolvemission)
ALTER TABLE public.evolvedomain RENAME CONSTRAINT evolvemission_pkey TO evolvedomain_pkey;

-- domainpack (was missionpack)
ALTER TABLE public.domainpack RENAME CONSTRAINT missionpack_pkey TO domainpack_pkey;
DO $$ BEGIN
    ALTER TABLE public.domainpack RENAME CONSTRAINT uq_missionpack_owner_name_version TO uq_domainpack_owner_name_version;
EXCEPTION WHEN undefined_object THEN NULL; END $$;

-- domainpersistencepolicy (was missionpersistencepolicy)
ALTER TABLE public.domainpersistencepolicy RENAME CONSTRAINT missionpersistencepolicy_pkey TO domainpersistencepolicy_pkey;
DO $$ BEGIN
    ALTER TABLE public.domainpersistencepolicy RENAME CONSTRAINT uq_mission_persistence_policy_mission TO uq_domain_persistence_policy_domain;
EXCEPTION WHEN undefined_object THEN NULL; END $$;

-- domainpersistenceroute (was missionpersistenceroute)
ALTER TABLE public.domainpersistenceroute RENAME CONSTRAINT missionpersistenceroute_pkey TO domainpersistenceroute_pkey;

-- ============================================================
-- 4. COLUMN RENAMES — tables with ONLY mission_id (→ domain_id)
-- ============================================================

ALTER TABLE public.approvalrequest         RENAME COLUMN mission_id TO domain_id;
ALTER TABLE public.evolvedomain            RENAME COLUMN mission_id TO domain_id;
ALTER TABLE public.evolverun               RENAME COLUMN mission_id TO domain_id;
ALTER TABLE public.meshagent               RENAME COLUMN mission_id TO domain_id;
ALTER TABLE public.domainpersistencepolicy RENAME COLUMN mission_id TO domain_id;
ALTER TABLE public.domainpersistenceroute  RENAME COLUMN mission_id TO domain_id;
ALTER TABLE public.domainrolemembership    RENAME COLUMN mission_id TO domain_id;
ALTER TABLE public.publicationrecord       RENAME COLUMN mission_id TO domain_id;
ALTER TABLE public.runtimejob              RENAME COLUMN mission_id TO domain_id;
ALTER TABLE public.slackchannelbinding     RENAME COLUMN mission_id TO domain_id;

-- ============================================================
-- 5. COLUMN RENAMES — tables with ONLY kluster_id (→ mission_id)
-- ============================================================

ALTER TABLE public.artifact       RENAME COLUMN kluster_id TO mission_id;
ALTER TABLE public.doc             RENAME COLUMN kluster_id TO mission_id;
ALTER TABLE public.epic            RENAME COLUMN kluster_id TO mission_id;
ALTER TABLE public.ingestionjob    RENAME COLUMN kluster_id TO mission_id;
ALTER TABLE public.task            RENAME COLUMN kluster_id TO mission_id;

-- Also: mission (was kluster) has mission_id pointing at domain. Rename it.
ALTER TABLE public.mission         RENAME COLUMN mission_id TO domain_id;

-- ============================================================
-- 6. COLUMN RENAMES — collision tables (mission_id FIRST, then kluster_id)
-- ============================================================

-- feedbackentry
ALTER TABLE public.feedbackentry  RENAME COLUMN mission_id TO domain_id;
ALTER TABLE public.feedbackentry  RENAME COLUMN kluster_id TO mission_id;

-- ledgerevent
ALTER TABLE public.ledgerevent    RENAME COLUMN mission_id TO domain_id;
ALTER TABLE public.ledgerevent    RENAME COLUMN kluster_id TO mission_id;

-- meshmessage
ALTER TABLE public.meshmessage    RENAME COLUMN mission_id TO domain_id;
ALTER TABLE public.meshmessage    RENAME COLUMN kluster_id TO mission_id;

-- meshtask
ALTER TABLE public.meshtask       RENAME COLUMN mission_id TO domain_id;
ALTER TABLE public.meshtask       RENAME COLUMN kluster_id TO mission_id;

-- skillbundle
ALTER TABLE public.skillbundle    RENAME COLUMN mission_id TO domain_id;
ALTER TABLE public.skillbundle    RENAME COLUMN kluster_id TO mission_id;

-- skilllocalstate
ALTER TABLE public.skilllocalstate RENAME COLUMN mission_id TO domain_id;
ALTER TABLE public.skilllocalstate RENAME COLUMN kluster_id TO mission_id;

-- skillsnapshot (also has mission_bundle_id / kluster_bundle_id collision)
ALTER TABLE public.skillsnapshot  RENAME COLUMN mission_id TO domain_id;
ALTER TABLE public.skillsnapshot  RENAME COLUMN kluster_id TO mission_id;
ALTER TABLE public.skillsnapshot  RENAME COLUMN mission_bundle_id TO domain_bundle_id;
ALTER TABLE public.skillsnapshot  RENAME COLUMN kluster_bundle_id TO mission_bundle_id;

-- usagerecord
ALTER TABLE public.usagerecord    RENAME COLUMN mission_id TO domain_id;
ALTER TABLE public.usagerecord    RENAME COLUMN kluster_id TO mission_id;

-- workspacelease
ALTER TABLE public.workspacelease RENAME COLUMN mission_id TO domain_id;
ALTER TABLE public.workspacelease RENAME COLUMN kluster_id TO mission_id;

-- ============================================================
-- 7. AGENT FK COLUMNS (added in migration 0010)
-- ============================================================

ALTER TABLE public.agent RENAME COLUMN home_mission_id    TO home_domain_id;
ALTER TABLE public.agent RENAME COLUMN current_mission_id TO current_domain_id;

-- Rename FK constraints (auto-named by 0010's ADD COLUMN REFERENCES)
DO $$ BEGIN
    ALTER TABLE public.agent RENAME CONSTRAINT agent_home_mission_id_fkey    TO agent_home_domain_id_fkey;
EXCEPTION WHEN undefined_object THEN NULL; END $$;
DO $$ BEGIN
    ALTER TABLE public.agent RENAME CONSTRAINT agent_current_mission_id_fkey TO agent_current_domain_id_fkey;
EXCEPTION WHEN undefined_object THEN NULL; END $$;

-- Rename indexes from migration 0010
ALTER INDEX IF EXISTS agent_home_mission_idx    RENAME TO agent_home_domain_idx;
ALTER INDEX IF EXISTS agent_current_mission_idx RENAME TO agent_current_domain_idx;

-- ============================================================
-- 8. INDEX RENAMES — domain (was mission) table
-- ============================================================

ALTER INDEX IF EXISTS ix_mission_name RENAME TO ix_domain_name;
ALTER INDEX IF EXISTS ix_mission_kind RENAME TO ix_domain_kind;

-- ============================================================
-- 9. INDEX RENAMES — mission (was kluster) table
-- ============================================================

-- ix_kluster_mission_id points at domain_id column (was mission_id)
ALTER INDEX IF EXISTS ix_kluster_mission_id RENAME TO ix_mission_domain_id;
ALTER INDEX IF EXISTS ix_kluster_name       RENAME TO ix_mission_name;

-- ============================================================
-- 10. INDEX RENAMES — other tables (old mission_id → domain_id)
-- ============================================================

ALTER INDEX IF EXISTS ix_approvalrequest_mission_id           RENAME TO ix_approvalrequest_domain_id;
ALTER INDEX IF EXISTS ix_evolvemission_created_at             RENAME TO ix_evolvedomain_created_at;
ALTER INDEX IF EXISTS ix_evolvemission_mission_id             RENAME TO ix_evolvedomain_domain_id;
ALTER INDEX IF EXISTS ix_evolvemission_owner_subject          RENAME TO ix_evolvedomain_owner_subject;
ALTER INDEX IF EXISTS ix_evolvemission_status                 RENAME TO ix_evolvedomain_status;
ALTER INDEX IF EXISTS ix_evolvemission_updated_at             RENAME TO ix_evolvedomain_updated_at;
ALTER INDEX IF EXISTS ix_evolverun_mission_id                 RENAME TO ix_evolverun_domain_id;
ALTER INDEX IF EXISTS ix_meshagent_mission_id                 RENAME TO ix_meshagent_domain_id;
ALTER INDEX IF EXISTS ix_missionpack_owner_subject            RENAME TO ix_domainpack_owner_subject;
ALTER INDEX IF EXISTS ix_missionpersistencepolicy_created_at         RENAME TO ix_domainpersistencepolicy_created_at;
ALTER INDEX IF EXISTS ix_missionpersistencepolicy_default_binding_id RENAME TO ix_domainpersistencepolicy_default_binding_id;
ALTER INDEX IF EXISTS ix_missionpersistencepolicy_fallback_mode      RENAME TO ix_domainpersistencepolicy_fallback_mode;
ALTER INDEX IF EXISTS ix_missionpersistencepolicy_mission_id         RENAME TO ix_domainpersistencepolicy_domain_id;
ALTER INDEX IF EXISTS ix_missionpersistencepolicy_require_approval   RENAME TO ix_domainpersistencepolicy_require_approval;
ALTER INDEX IF EXISTS ix_missionpersistenceroute_active              RENAME TO ix_domainpersistenceroute_active;
ALTER INDEX IF EXISTS ix_missionpersistenceroute_binding_id          RENAME TO ix_domainpersistenceroute_binding_id;
ALTER INDEX IF EXISTS ix_missionpersistenceroute_created_at          RENAME TO ix_domainpersistenceroute_created_at;
ALTER INDEX IF EXISTS ix_missionpersistenceroute_entity_kind         RENAME TO ix_domainpersistenceroute_entity_kind;
ALTER INDEX IF EXISTS ix_missionpersistenceroute_event_kind          RENAME TO ix_domainpersistenceroute_event_kind;
ALTER INDEX IF EXISTS ix_missionpersistenceroute_mission_id          RENAME TO ix_domainpersistenceroute_domain_id;
ALTER INDEX IF EXISTS ix_missionrolemembership_created_at            RENAME TO ix_domainrolemembership_created_at;
ALTER INDEX IF EXISTS ix_missionrolemembership_mission_id            RENAME TO ix_domainrolemembership_domain_id;
ALTER INDEX IF EXISTS ix_missionrolemembership_role                  RENAME TO ix_domainrolemembership_role;
ALTER INDEX IF EXISTS ix_missionrolemembership_subject               RENAME TO ix_domainrolemembership_subject;
ALTER INDEX IF EXISTS ix_publicationrecord_mission_id         RENAME TO ix_publicationrecord_domain_id;
ALTER INDEX IF EXISTS ix_runtimejob_mission_id                RENAME TO ix_runtimejob_domain_id;
ALTER INDEX IF EXISTS ix_slackchannelbinding_mission_id       RENAME TO ix_slackchannelbinding_domain_id;

-- ============================================================
-- 11. INDEX RENAMES — other tables (old kluster_id → mission_id)
--     Collision tables: rename old _mission_id index FIRST, then old _kluster_id
-- ============================================================

-- artifact
ALTER INDEX IF EXISTS ix_artifact_kluster_id       RENAME TO ix_artifact_mission_id;

-- doc
ALTER INDEX IF EXISTS ix_doc_kluster_id            RENAME TO ix_doc_mission_id;

-- epic
ALTER INDEX IF EXISTS ix_epic_kluster_id           RENAME TO ix_epic_mission_id;

-- feedbackentry (collision: rename _mission_id first, then _kluster_id)
ALTER INDEX IF EXISTS ix_feedbackentry_mission_id  RENAME TO ix_feedbackentry_domain_id;
ALTER INDEX IF EXISTS ix_feedbackentry_kluster_id  RENAME TO ix_feedbackentry_mission_id;

-- ingestionjob
ALTER INDEX IF EXISTS ix_ingestionjob_kluster_id   RENAME TO ix_ingestionjob_mission_id;

-- ledgerevent (collision)
ALTER INDEX IF EXISTS ix_ledgerevent_mission_id    RENAME TO ix_ledgerevent_domain_id;
ALTER INDEX IF EXISTS ix_ledgerevent_kluster_id    RENAME TO ix_ledgerevent_mission_id;

-- meshmessage (collision)
ALTER INDEX IF EXISTS ix_meshmessage_mission_id    RENAME TO ix_meshmessage_domain_id;
ALTER INDEX IF EXISTS ix_meshmessage_kluster_id    RENAME TO ix_meshmessage_mission_id;

-- meshtask (collision)
ALTER INDEX IF EXISTS ix_meshtask_mission_id       RENAME TO ix_meshtask_domain_id;
ALTER INDEX IF EXISTS ix_meshtask_kluster_id       RENAME TO ix_meshtask_mission_id;

-- skillbundle (collision)
ALTER INDEX IF EXISTS ix_skillbundle_mission_id    RENAME TO ix_skillbundle_domain_id;
ALTER INDEX IF EXISTS ix_skillbundle_kluster_id    RENAME TO ix_skillbundle_mission_id;

-- skilllocalstate (collision)
ALTER INDEX IF EXISTS ix_skilllocalstate_mission_id RENAME TO ix_skilllocalstate_domain_id;
ALTER INDEX IF EXISTS ix_skilllocalstate_kluster_id RENAME TO ix_skilllocalstate_mission_id;

-- skillsnapshot (collision, including bundle cols)
ALTER INDEX IF EXISTS ix_skillsnapshot_mission_id         RENAME TO ix_skillsnapshot_domain_id;
ALTER INDEX IF EXISTS ix_skillsnapshot_kluster_id         RENAME TO ix_skillsnapshot_mission_id;
ALTER INDEX IF EXISTS ix_skillsnapshot_mission_bundle_id  RENAME TO ix_skillsnapshot_domain_bundle_id;
ALTER INDEX IF EXISTS ix_skillsnapshot_kluster_bundle_id  RENAME TO ix_skillsnapshot_mission_bundle_id;

-- task
ALTER INDEX IF EXISTS ix_task_kluster_id           RENAME TO ix_task_mission_id;

-- workspacelease (collision)
ALTER INDEX IF EXISTS ix_workspacelease_mission_id RENAME TO ix_workspacelease_domain_id;
ALTER INDEX IF EXISTS ix_workspacelease_kluster_id RENAME TO ix_workspacelease_mission_id;

COMMIT;

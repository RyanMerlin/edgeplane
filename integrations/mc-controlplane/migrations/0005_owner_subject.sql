-- Migration 0005: add owner_subject / created_by_subject columns
--
-- These columns were added in-place to 0001_initial_schema.sql; this migration
-- makes existing databases (deployed before the column was introduced) forward-
-- compatible without destroying data.
--
-- All ADD COLUMN statements use IF NOT EXISTS so the migration is safe to re-run.
-- Indexes use CREATE INDEX IF NOT EXISTS for the same reason.
-- Unique constraints use the DO $$ BEGIN … EXCEPTION WHEN OTHERS THEN NULL; END $$
-- pattern so they're idempotent even when PostgreSQL doesn't support
-- ADD CONSTRAINT IF NOT EXISTS for unique constraints.

-- ---------------------------------------------------------------------------
-- owner_subject column — 21 tables
-- ---------------------------------------------------------------------------

ALTER TABLE public.agentrun           ADD COLUMN IF NOT EXISTS owner_subject character varying NOT NULL DEFAULT '';
ALTER TABLE public.aisession           ADD COLUMN IF NOT EXISTS owner_subject character varying NOT NULL DEFAULT '';
ALTER TABLE public.budgetpolicy        ADD COLUMN IF NOT EXISTS owner_subject character varying NOT NULL DEFAULT '';
ALTER TABLE public.eventtrigger        ADD COLUMN IF NOT EXISTS owner_subject character varying NOT NULL DEFAULT '';
ALTER TABLE public.evolvemission       ADD COLUMN IF NOT EXISTS owner_subject character varying NOT NULL DEFAULT '';
ALTER TABLE public.evolverun           ADD COLUMN IF NOT EXISTS owner_subject character varying NOT NULL DEFAULT '';
ALTER TABLE public.missionpack         ADD COLUMN IF NOT EXISTS owner_subject character varying NOT NULL DEFAULT '';
ALTER TABLE public.publicationrecord   ADD COLUMN IF NOT EXISTS owner_subject character varying NOT NULL DEFAULT '';
ALTER TABLE public.remotelaunchrecord  ADD COLUMN IF NOT EXISTS owner_subject character varying NOT NULL DEFAULT '';
ALTER TABLE public.remotetarget        ADD COLUMN IF NOT EXISTS owner_subject character varying NOT NULL DEFAULT '';
ALTER TABLE public.repobinding         ADD COLUMN IF NOT EXISTS owner_subject character varying NOT NULL DEFAULT '';
ALTER TABLE public.repoconnection      ADD COLUMN IF NOT EXISTS owner_subject character varying NOT NULL DEFAULT '';
ALTER TABLE public.reviewgate          ADD COLUMN IF NOT EXISTS owner_subject character varying NOT NULL DEFAULT '';
ALTER TABLE public.runtimejob          ADD COLUMN IF NOT EXISTS owner_subject character varying NOT NULL DEFAULT '';
ALTER TABLE public.runtimejointoken    ADD COLUMN IF NOT EXISTS owner_subject character varying NOT NULL DEFAULT '';
ALTER TABLE public.runtimenode         ADD COLUMN IF NOT EXISTS owner_subject character varying NOT NULL DEFAULT '';
ALTER TABLE public.runtimenodespec     ADD COLUMN IF NOT EXISTS owner_subject character varying NOT NULL DEFAULT '';
ALTER TABLE public.scheduledagentjob   ADD COLUMN IF NOT EXISTS owner_subject character varying NOT NULL DEFAULT '';
ALTER TABLE public.serviceaccount      ADD COLUMN IF NOT EXISTS owner_subject character varying NOT NULL DEFAULT '';
ALTER TABLE public.usagerecord         ADD COLUMN IF NOT EXISTS owner_subject character varying NOT NULL DEFAULT '';
ALTER TABLE public.userprofile         ADD COLUMN IF NOT EXISTS owner_subject character varying NOT NULL DEFAULT '';

-- ---------------------------------------------------------------------------
-- created_by_subject column — 2 tables
-- ---------------------------------------------------------------------------

ALTER TABLE public.ledgerevent  ADD COLUMN IF NOT EXISTS created_by_subject character varying NOT NULL DEFAULT '';
ALTER TABLE public.meshtask     ADD COLUMN IF NOT EXISTS created_by_subject character varying NOT NULL DEFAULT '';

-- ---------------------------------------------------------------------------
-- Indexes on owner_subject
-- ---------------------------------------------------------------------------

CREATE INDEX IF NOT EXISTS ix_agentrun_owner_subject          ON public.agentrun          USING btree (owner_subject);
CREATE INDEX IF NOT EXISTS ix_aisession_owner_subject         ON public.aisession          USING btree (owner_subject);
CREATE INDEX IF NOT EXISTS ix_budgetpolicy_owner_subject      ON public.budgetpolicy       USING btree (owner_subject);
CREATE INDEX IF NOT EXISTS ix_eventtrigger_owner_subject      ON public.eventtrigger       USING btree (owner_subject);
CREATE INDEX IF NOT EXISTS ix_evolvemission_owner_subject     ON public.evolvemission      USING btree (owner_subject);
CREATE INDEX IF NOT EXISTS ix_evolverun_owner_subject         ON public.evolverun          USING btree (owner_subject);
CREATE INDEX IF NOT EXISTS ix_missionpack_owner_subject       ON public.missionpack        USING btree (owner_subject);
CREATE INDEX IF NOT EXISTS ix_publicationrecord_owner_subject ON public.publicationrecord  USING btree (owner_subject);
CREATE INDEX IF NOT EXISTS ix_remotelaunchrecord_owner_subject ON public.remotelaunchrecord USING btree (owner_subject);
CREATE INDEX IF NOT EXISTS ix_remotetarget_owner_subject      ON public.remotetarget       USING btree (owner_subject);
CREATE INDEX IF NOT EXISTS ix_repobinding_owner_subject       ON public.repobinding        USING btree (owner_subject);
CREATE INDEX IF NOT EXISTS ix_repoconnection_owner_subject    ON public.repoconnection     USING btree (owner_subject);
CREATE INDEX IF NOT EXISTS ix_reviewgate_owner_subject        ON public.reviewgate         USING btree (owner_subject);
CREATE INDEX IF NOT EXISTS ix_runtimejob_owner_subject        ON public.runtimejob         USING btree (owner_subject);
CREATE INDEX IF NOT EXISTS ix_runtimejointoken_owner_subject  ON public.runtimejointoken   USING btree (owner_subject);
CREATE INDEX IF NOT EXISTS ix_runtimenode_owner_subject       ON public.runtimenode        USING btree (owner_subject);
CREATE INDEX IF NOT EXISTS ix_runtimenodespec_owner_subject   ON public.runtimenodespec    USING btree (owner_subject);
CREATE INDEX IF NOT EXISTS ix_scheduledagentjob_owner_subject ON public.scheduledagentjob  USING btree (owner_subject);
CREATE INDEX IF NOT EXISTS ix_serviceaccount_owner_subject    ON public.serviceaccount     USING btree (owner_subject);
CREATE INDEX IF NOT EXISTS ix_usagerecord_owner_subject       ON public.usagerecord        USING btree (owner_subject);
CREATE INDEX IF NOT EXISTS ix_userprofile_owner_subject       ON public.userprofile        USING btree (owner_subject);

-- ---------------------------------------------------------------------------
-- Indexes on created_by_subject
-- ---------------------------------------------------------------------------

CREATE INDEX IF NOT EXISTS ix_ledgerevent_created_by_subject ON public.ledgerevent USING btree (created_by_subject);
CREATE INDEX IF NOT EXISTS ix_meshtask_created_by_subject    ON public.meshtask    USING btree (created_by_subject);

-- ---------------------------------------------------------------------------
-- Unique constraints that include owner_subject
-- ---------------------------------------------------------------------------

DO $$ BEGIN
    ALTER TABLE ONLY public.agentrun    ADD CONSTRAINT uq_agentrun_owner_idempotency    UNIQUE (owner_subject, idempotency_key);
EXCEPTION WHEN others THEN NULL; END $$;

DO $$ BEGIN
    ALTER TABLE ONLY public.missionpack ADD CONSTRAINT uq_missionpack_owner_name_version UNIQUE (owner_subject, name, version);
EXCEPTION WHEN others THEN NULL; END $$;

DO $$ BEGIN
    ALTER TABLE ONLY public.remotetarget ADD CONSTRAINT uq_remotetarget_owner_name UNIQUE (owner_subject, name);
EXCEPTION WHEN others THEN NULL; END $$;

DO $$ BEGIN
    ALTER TABLE ONLY public.repobinding ADD CONSTRAINT uq_repo_binding_owner_name UNIQUE (owner_subject, name);
EXCEPTION WHEN others THEN NULL; END $$;

DO $$ BEGIN
    ALTER TABLE ONLY public.repoconnection ADD CONSTRAINT uq_repo_connection_owner_name UNIQUE (owner_subject, name);
EXCEPTION WHEN others THEN NULL; END $$;

DO $$ BEGIN
    ALTER TABLE ONLY public.userprofile ADD CONSTRAINT uq_userprofile_owner_name UNIQUE (owner_subject, name);
EXCEPTION WHEN others THEN NULL; END $$;

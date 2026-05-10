DO $$ BEGIN ALTER TABLE public.runtimenode ADD COLUMN attach_secret character varying; EXCEPTION WHEN duplicate_column THEN NULL; END $$;

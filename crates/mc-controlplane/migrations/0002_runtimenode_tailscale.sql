DO $$ BEGIN ALTER TABLE public.runtimenode ADD COLUMN tailscale_ip character varying; EXCEPTION WHEN duplicate_column THEN NULL; END $$;
DO $$ BEGIN ALTER TABLE public.runtimenode ADD COLUMN tailscale_fqdn character varying; EXCEPTION WHEN duplicate_column THEN NULL; END $$;

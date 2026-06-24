-- Carry the IdP display name (preferred_username / name) on the login grant so
-- the CLI/device token exchange can persist it on the session — the browser path
-- already captured it, but exchange_grant/device_token read only the grant row
-- and previously passed display_name=NULL, leaving `whoami` / "Logged in as"
-- blank for CLI logins. Additive + nullable — safe on existing rows.
ALTER TABLE public.oidclogingrant ADD COLUMN IF NOT EXISTS display_name character varying;

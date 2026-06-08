-- Add nullable display_name to usersession, populated from the OIDC
-- preferred_username/name claim at login. Used by /auth/me to give the web UI
-- a human-readable label for the avatar. Additive + nullable — safe on existing rows.
ALTER TABLE public.usersession ADD COLUMN IF NOT EXISTS display_name character varying;

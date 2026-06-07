-- Add nullable email to usersession so /auth/me can expose the human identity
-- (web avatar initials). Populated for browser OIDC logins; NULL for CLI/SA/device
-- sessions. Additive + nullable — safe on existing rows.
ALTER TABLE public.usersession ADD COLUMN IF NOT EXISTS email character varying;

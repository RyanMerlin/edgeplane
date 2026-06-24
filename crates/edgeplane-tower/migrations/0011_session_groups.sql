-- Group-based admin (#77 follow-up): persist the IdP `groups` claim on the
-- login grant and the session so request-time authz can derive admin from group
-- membership (EP_ADMIN_GROUPS), not only the email allowlist. Stored as a JSON
-- array of group-name strings; '[]' for sessions with no groups (CLI/SA/device,
-- or an IdP that doesn't emit groups). Additive + NOT NULL DEFAULT — safe on
-- existing rows and on inserts that omit the column.
ALTER TABLE public.oidclogingrant ADD COLUMN IF NOT EXISTS groups text NOT NULL DEFAULT '[]';
ALTER TABLE public.usersession   ADD COLUMN IF NOT EXISTS groups text NOT NULL DEFAULT '[]';

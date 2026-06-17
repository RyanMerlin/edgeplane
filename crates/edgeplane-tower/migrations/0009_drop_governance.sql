-- R1d governance simplification: drop the never-enforced policy/approvals/family
-- scaffolding and the half-wired domain role-membership table. All five are dead,
-- dormant, or half-broken; no code references them after this migration's PR.
-- Destructive and effectively irreversible (a down-migration would recreate empty
-- tables, not restore rows). DROP TABLE IF EXISTS is idempotent and safe on both
-- fresh and existing databases.
DROP TABLE IF EXISTS public.governancepolicy;
DROP TABLE IF EXISTS public.governancepolicyevent;
DROP TABLE IF EXISTS public.approvalrequest;
DROP TABLE IF EXISTS public.familymember;
DROP TABLE IF EXISTS public.domainrolemembership;

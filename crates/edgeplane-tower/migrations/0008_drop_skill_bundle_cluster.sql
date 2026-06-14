-- R1c (full): remove the fork-inherited, unused skill-bundle + domain-pack cluster.
-- The skill-sync delivery (skilllocalstate/skillsnapshot) plus the bundle storage
-- (skillbundle) and the domain export/import packs (domainpack). No remaining code
-- references these; no inbound FKs.
DROP TABLE IF EXISTS public.skilllocalstate;
DROP TABLE IF EXISTS public.skillsnapshot;
DROP TABLE IF EXISTS public.domainpack;
DROP TABLE IF EXISTS public.skillbundle;

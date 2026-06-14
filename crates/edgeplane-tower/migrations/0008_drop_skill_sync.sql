-- R1c: remove the dormant skill-sync delivery layer. skillbundle is KEPT
-- (domain-packs read/write it); only the never-consumed snapshot/localstate
-- delivery tables are dropped. No kept code references these after this change.
DROP TABLE IF EXISTS public.skilllocalstate;
DROP TABLE IF EXISTS public.skillsnapshot;

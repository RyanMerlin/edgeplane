-- R1b: remove the dead+broken `evolve` feature.
-- The tower routes INSERTed into a non-existent `evolvemission` table (Mission->Domain
-- rename left the SQL stale), so the write path never succeeded; evolvedomain/evolverun
-- are empty by construction and have no inbound FKs.
DROP TABLE IF EXISTS public.evolverun;
DROP TABLE IF EXISTS public.evolvedomain;

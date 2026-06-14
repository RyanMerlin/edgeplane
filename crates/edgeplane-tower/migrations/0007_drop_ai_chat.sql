-- R1a: remove the built-in AI chat. No kept table references these
-- (the only ai_session_id column was on evolverun, dropped in 0006).
DROP TABLE IF EXISTS public.aiturn;
DROP TABLE IF EXISTS public.aievent;
DROP TABLE IF EXISTS public.aipendingaction;
DROP TABLE IF EXISTS public.aisession;

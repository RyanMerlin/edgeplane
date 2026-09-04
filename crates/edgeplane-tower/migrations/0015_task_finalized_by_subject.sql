-- Attribution for a task's terminal transition (complete/fail, and later
-- resolve_gate). complete_task/fail_task now clear claimed_by_agent_id on
-- terminal transition to close a real ownership-carryover gap (see EP-1
-- tower-fencing plan), which destroys the audit record of who actually did
-- the work. finalized_by_subject preserves that identity independently and
-- is never consulted by any authorization predicate.
ALTER TABLE public.task
    ADD COLUMN IF NOT EXISTS finalized_by_subject character varying;

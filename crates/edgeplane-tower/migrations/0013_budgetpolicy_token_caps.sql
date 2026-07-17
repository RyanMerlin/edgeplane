-- Add nullable token_hard_cap/token_soft_cap to budgetpolicy. The columns
-- were referenced by routes/budgets.rs (BudgetPolicyCreate, row_to_policy,
-- the INSERT in create_budget_policy) since the pre-fork MissionControl
-- codebase but were never migrated — every POST/GET /budgets call has always
-- failed with "column does not exist". Additive + nullable — safe on
-- existing rows.
ALTER TABLE public.budgetpolicy ADD COLUMN IF NOT EXISTS token_hard_cap integer;
ALTER TABLE public.budgetpolicy ADD COLUMN IF NOT EXISTS token_soft_cap integer;

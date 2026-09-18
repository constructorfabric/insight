-- Drop `issue_kind` from the issue-type class dimension.
--
-- Classification moved to gold: `task_issue_state` resolves the kind from
-- `config.field_value_map` (falling to `config.field_value_defaults`, then
-- 'unknown') at its own build, so an operator decision applies at the next
-- gold run without a silver rebuild. Silver keeps raw vendor identity only.
--
-- The staging projections (`staging.jira__task_issuetypes`,
-- `staging.github__task_issuetypes`) drop the column in the same change but
-- need no heal here: both are views, recreated by dbt at the connector's next
-- run. Until then the class model's positional `union_by_tag` insert fails
-- closed rather than misplacing a value, and converges at that run.
--
-- Dropping a column keeps the relative order of the remaining ones, which is
-- what the model's SELECT now emits — the positional contract holds without a
-- MODIFY COLUMN. `IF EXISTS` makes a re-run (and a fresh install, whose
-- snapshot may already lack the column) a no-op.

ALTER TABLE silver.class_task_issuetypes
    DROP COLUMN IF EXISTS issue_kind;

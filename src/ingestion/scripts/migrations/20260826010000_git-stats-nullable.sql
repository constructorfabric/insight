-- Widen the git class contract's size columns to the Nullable type every
-- staging projection emits.
--
-- The projections wrap these in toNullable(), so the type the model produces
-- is Nullable(Int64). A relation created before that carries the narrower
-- non-null type ClickHouse inferred from the values it held at creation, and
-- the two never converge on their own: the DDL snapshot is IF NOT EXISTS, and
-- on_schema_change='append_new_columns' adds columns but cannot reconcile a
-- changed type. dbt compares source against target on every incremental build
-- and aborts the model when they differ, so a warm relation stops building and
-- its class freezes at the last successful run while its neighbours keep
-- moving. A warehouse built from scratch never reaches that state: the table
-- is created from the model, so the types already agree.
--
-- Type only, no AFTER anchor — the columns already sit at their contract
-- positions and the positional insert needs them left alone. Shape follows the
-- retype heals in apply-ch-migrations.sh, where the staging halves converge.
--
-- Idempotent: this channel has no ledger and re-runs on every deploy.
-- The class tables always exist here (placeholders precede migrations).
ALTER TABLE silver.class_git_commits
    MODIFY COLUMN IF EXISTS files_changed Nullable(Int64);

ALTER TABLE silver.class_git_commits
    MODIFY COLUMN IF EXISTS lines_added Nullable(Int64);

ALTER TABLE silver.class_git_commits
    MODIFY COLUMN IF EXISTS lines_removed Nullable(Int64);

ALTER TABLE silver.class_git_file_changes
    MODIFY COLUMN IF EXISTS lines_added Nullable(Int64);

ALTER TABLE silver.class_git_file_changes
    MODIFY COLUMN IF EXISTS lines_removed Nullable(Int64);

ALTER TABLE silver.class_git_pull_requests
    MODIFY COLUMN IF EXISTS files_changed Nullable(Int64);

ALTER TABLE silver.class_git_pull_requests
    MODIFY COLUMN IF EXISTS lines_added Nullable(Int64);

ALTER TABLE silver.class_git_pull_requests
    MODIFY COLUMN IF EXISTS lines_removed Nullable(Int64);

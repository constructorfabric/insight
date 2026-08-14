-- Add is_deleted to the worklog class contract (jira worklog deletion
-- reconciliation, #2419).
--
-- dbt-clickhouse incremental inserts are positional, so the physical column
-- must sit exactly where the model's SELECT emits it: after collected_at,
-- before _version. New bronze tables (jira_worklog_deleted, censuses) need no
-- migration — the ClickHouse destination creates missing tables on first sync
-- and bootstrap-db creates them from the connectors-ddl snapshot.
--
-- Idempotent: this channel has no ledger and re-runs on every deploy.
-- Both tables always exist here: silver placeholders precede migrations and
-- the staging table is created by the dbt run of the same deploy.

ALTER TABLE staging.jira__task_worklogs ADD COLUMN IF NOT EXISTS is_deleted Nullable(UInt8) AFTER collected_at;

ALTER TABLE silver.class_task_worklogs ADD COLUMN IF NOT EXISTS is_deleted Nullable(UInt8) AFTER collected_at;

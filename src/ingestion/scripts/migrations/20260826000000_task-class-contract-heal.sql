-- Converge the task class relations on their contract column set.
--
-- The DDL snapshot only CREATEs IF NOT EXISTS and the deploy hook builds
-- tag:gold, so a relation that already holds data keeps whatever column list
-- it was created with: gold fails resolving `is_deleted` on class_task_worklogs,
-- and the columns added to the class contracts since never arrive.
--
-- dbt-clickhouse incremental inserts are positional and union_by_tag is a
-- positional SELECT * UNION ALL, so physical column order must equal the
-- staging projections' SELECT order. ADD places a missing column correctly;
-- MODIFY moves one an earlier out-of-band ADD appended at the tail. DROP
-- removes what left the contract, preserving the order of what remains.
--
-- Idempotent: this channel has no ledger and re-runs on every deploy.
-- The class tables always exist here (placeholders precede migrations).
ALTER TABLE silver.class_task_worklogs DROP COLUMN IF EXISTS insight_tenant_id;
ALTER TABLE silver.class_task_worklogs DROP COLUMN IF EXISTS issue_id;
ALTER TABLE silver.class_task_worklogs DROP COLUMN IF EXISTS author_email;
ALTER TABLE silver.class_task_worklogs DROP COLUMN IF EXISTS worklog_seconds;

ALTER TABLE silver.class_task_worklogs
    ADD COLUMN IF NOT EXISTS data_source String AFTER insight_source_id;
ALTER TABLE silver.class_task_worklogs
    MODIFY COLUMN data_source String AFTER insight_source_id;

ALTER TABLE silver.class_task_worklogs
    ADD COLUMN IF NOT EXISTS id_readable Nullable(String) AFTER worklog_id;
ALTER TABLE silver.class_task_worklogs
    MODIFY COLUMN id_readable Nullable(String) AFTER worklog_id;

ALTER TABLE silver.class_task_worklogs
    ADD COLUMN IF NOT EXISTS description Nullable(String) AFTER duration_seconds;
ALTER TABLE silver.class_task_worklogs
    MODIFY COLUMN description Nullable(String) AFTER duration_seconds;

ALTER TABLE silver.class_task_worklogs
    ADD COLUMN IF NOT EXISTS collected_at Nullable(DateTime64(3)) AFTER description;
ALTER TABLE silver.class_task_worklogs
    MODIFY COLUMN collected_at Nullable(DateTime64(3)) AFTER description;

ALTER TABLE silver.class_task_worklogs
    ADD COLUMN IF NOT EXISTS is_deleted Nullable(UInt8) AFTER collected_at;
ALTER TABLE silver.class_task_worklogs
    MODIFY COLUMN is_deleted Nullable(UInt8) AFTER collected_at;

ALTER TABLE silver.class_task_users DROP COLUMN IF EXISTS insight_tenant_id;

ALTER TABLE silver.class_task_users
    ADD COLUMN IF NOT EXISTS data_source String AFTER insight_source_id;
ALTER TABLE silver.class_task_users
    MODIFY COLUMN data_source String AFTER insight_source_id;

ALTER TABLE silver.class_task_users
    ADD COLUMN IF NOT EXISTS display_name Nullable(String) AFTER email;
ALTER TABLE silver.class_task_users
    MODIFY COLUMN display_name Nullable(String) AFTER email;

ALTER TABLE silver.class_task_users
    ADD COLUMN IF NOT EXISTS username Nullable(String) AFTER display_name;
ALTER TABLE silver.class_task_users
    MODIFY COLUMN username Nullable(String) AFTER display_name;

ALTER TABLE silver.class_task_users
    ADD COLUMN IF NOT EXISTS account_type Nullable(String) AFTER username;
ALTER TABLE silver.class_task_users
    MODIFY COLUMN account_type Nullable(String) AFTER username;

ALTER TABLE silver.class_task_users
    ADD COLUMN IF NOT EXISTS is_active Nullable(UInt8) AFTER account_type;
ALTER TABLE silver.class_task_users
    MODIFY COLUMN is_active Nullable(UInt8) AFTER account_type;

ALTER TABLE silver.class_task_users
    ADD COLUMN IF NOT EXISTS collected_at DateTime64(3) AFTER is_active;
ALTER TABLE silver.class_task_users
    MODIFY COLUMN collected_at DateTime64(3) AFTER is_active;

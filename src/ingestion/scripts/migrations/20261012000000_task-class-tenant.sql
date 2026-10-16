ALTER TABLE silver.class_task_comments
    ADD COLUMN IF NOT EXISTS tenant_id Nullable(String) AFTER unique_key;
ALTER TABLE silver.class_task_comments
    MODIFY COLUMN tenant_id Nullable(String) AFTER unique_key;

ALTER TABLE silver.class_task_field_history
    ADD COLUMN IF NOT EXISTS tenant_id Nullable(String) AFTER unique_key;
ALTER TABLE silver.class_task_field_history
    MODIFY COLUMN tenant_id Nullable(String) AFTER unique_key;

ALTER TABLE silver.class_task_field_metadata
    ADD COLUMN IF NOT EXISTS tenant_id Nullable(String) AFTER unique_key;
ALTER TABLE silver.class_task_field_metadata
    MODIFY COLUMN tenant_id Nullable(String) AFTER unique_key;

ALTER TABLE silver.class_task_issuetypes
    ADD COLUMN IF NOT EXISTS tenant_id Nullable(String) AFTER unique_key;
ALTER TABLE silver.class_task_issuetypes
    MODIFY COLUMN tenant_id Nullable(String) AFTER unique_key;

ALTER TABLE silver.class_task_links
    ADD COLUMN IF NOT EXISTS tenant_id Nullable(String) AFTER unique_key;
ALTER TABLE silver.class_task_links
    MODIFY COLUMN tenant_id Nullable(String) AFTER unique_key;

ALTER TABLE silver.class_task_projects
    ADD COLUMN IF NOT EXISTS tenant_id Nullable(String) AFTER unique_key;
ALTER TABLE silver.class_task_projects
    MODIFY COLUMN tenant_id Nullable(String) AFTER unique_key;

ALTER TABLE silver.class_task_sprints
    ADD COLUMN IF NOT EXISTS tenant_id Nullable(String) AFTER unique_key;
ALTER TABLE silver.class_task_sprints
    MODIFY COLUMN tenant_id Nullable(String) AFTER unique_key;

ALTER TABLE silver.class_task_statuses
    ADD COLUMN IF NOT EXISTS tenant_id Nullable(String) AFTER unique_key;
ALTER TABLE silver.class_task_statuses
    MODIFY COLUMN tenant_id Nullable(String) AFTER unique_key;

ALTER TABLE silver.class_task_worklogs
    ADD COLUMN IF NOT EXISTS tenant_id Nullable(String) AFTER unique_key;
ALTER TABLE silver.class_task_worklogs
    MODIFY COLUMN tenant_id Nullable(String) AFTER unique_key;

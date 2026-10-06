ALTER TABLE silver.class_task_comments
    ADD COLUMN IF NOT EXISTS issue_id Nullable(String) AFTER _version;
ALTER TABLE silver.class_task_comments
    MODIFY COLUMN issue_id Nullable(String) AFTER _version;

ALTER TABLE silver.class_task_worklogs
    ADD COLUMN IF NOT EXISTS issue_id Nullable(String) AFTER _version;
ALTER TABLE silver.class_task_worklogs
    MODIFY COLUMN issue_id Nullable(String) AFTER _version;

ALTER TABLE silver.class_task_links
    ADD COLUMN IF NOT EXISTS issue_id Nullable(String) AFTER _version;
ALTER TABLE silver.class_task_links
    ADD COLUMN IF NOT EXISTS target_id Nullable(String) AFTER issue_id;
ALTER TABLE silver.class_task_links
    MODIFY COLUMN issue_id Nullable(String) AFTER _version;
ALTER TABLE silver.class_task_links
    MODIFY COLUMN target_id Nullable(String) AFTER issue_id;

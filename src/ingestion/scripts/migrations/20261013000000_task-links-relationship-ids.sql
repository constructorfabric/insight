-- INVARIANT: union_by_tag inserts positionally, so issue_id and target_id sit after _version, as every producer emits them.

ALTER TABLE silver.class_task_links
    ADD COLUMN IF NOT EXISTS issue_id Nullable(String) AFTER _version;
ALTER TABLE silver.class_task_links
    ADD COLUMN IF NOT EXISTS target_id Nullable(String) AFTER issue_id;
ALTER TABLE silver.class_task_links
    MODIFY COLUMN issue_id Nullable(String) AFTER _version;
ALTER TABLE silver.class_task_links
    MODIFY COLUMN target_id Nullable(String) AFTER issue_id;

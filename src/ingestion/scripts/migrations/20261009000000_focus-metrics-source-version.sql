-- Focus rows record the newest meeting-row version they were derived from, so an
-- incremental build re-derives exactly the person-days whose meetings changed.
-- Rows stored before this column existed read 0 and are re-derived once.
-- Idempotent: this channel has no ledger and re-runs on every deploy.

ALTER TABLE silver.class_focus_metrics
    ADD COLUMN IF NOT EXISTS source_version Int64 AFTER _version;

ALTER TABLE silver.class_focus_metrics
    MODIFY COLUMN source_version Int64 AFTER _version;

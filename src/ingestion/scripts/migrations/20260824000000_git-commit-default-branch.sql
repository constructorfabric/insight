-- The class model owns this column but never adds it to a warm relation: the
-- DDL snapshot is IF NOT EXISTS, and the deploy hook builds tag:gold, not the
-- tag:silver model that would widen it — so gold reads it in the same run.
-- AFTER anchors the contract position the positional insert requires; MODIFY
-- converges an instance where an out-of-band ALTER placed it elsewhere.
-- Idempotent: this channel has no ledger and re-runs on every deploy.
ALTER TABLE silver.class_git_commits
    ADD COLUMN IF NOT EXISTS is_default_branch Nullable(UInt8) AFTER branch;

ALTER TABLE silver.class_git_commits
    MODIFY COLUMN is_default_branch Nullable(UInt8) AFTER branch;

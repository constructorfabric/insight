-- The class model owns this column but never adds it to a warm relation: the
-- DDL snapshot is IF NOT EXISTS, and the deploy hook builds tag:gold, not the
-- tag:silver model that would widen it — so gold reads it in the same run.
-- AFTER anchors the contract position the positional insert requires; MODIFY
-- converges an instance where an out-of-band ALTER placed it elsewhere.
-- Idempotent: this channel has no ledger and re-runs on every deploy.
ALTER TABLE silver.class_git_pull_requests
    ADD COLUMN IF NOT EXISTS author_account_id String AFTER author_email;

ALTER TABLE silver.class_git_pull_requests
    MODIFY COLUMN author_account_id String AFTER author_email;

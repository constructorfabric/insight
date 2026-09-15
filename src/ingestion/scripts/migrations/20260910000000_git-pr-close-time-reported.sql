-- The class model owns this column but never adds it to a warm relation: the
-- DDL snapshot is IF NOT EXISTS, and the deploy hook builds tag:gold, not the
-- tag:silver model that would widen it — so gold reads it in the same run.
-- AFTER anchors the contract position the positional insert requires; MODIFY
-- converges an instance where an out-of-band ALTER placed it elsewhere.
-- Idempotent: this channel has no ledger and re-runs on every deploy.
--
ALTER TABLE silver.class_git_pull_requests
    ADD COLUMN IF NOT EXISTS closed_on_reported Nullable(DateTime) AFTER closed_on;

ALTER TABLE silver.class_git_pull_requests
    MODIFY COLUMN closed_on_reported Nullable(DateTime) AFTER closed_on;

-- Filling the column for the rows a warm warehouse already holds needs the
-- SOURCE timestamps, not this table's settled `closed_on`, and one of the
-- relations it reads is only present on an installation that predates #3250 —
-- so the backfill lives in apply-ch-migrations.sh beside the staging heal,
-- where a missing relation can be guarded. #3362

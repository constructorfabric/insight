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

-- GitHub and GitLab report the close time themselves, so an existing row's
-- `closed_on` IS the source's own instant: copying it keeps every historical
-- duration observation instead of blanking the interval measures between this
-- deploy and the next sync. Bitbucket is excluded by design — its `closed_on`
-- may be RECOVERED, and a recovered time posing as a reported one is the error
-- this column exists to prevent, so its rows stay NULL and contribute no
-- duration. #3362
ALTER TABLE silver.class_git_pull_requests
    UPDATE closed_on_reported = closed_on
    WHERE closed_on_reported IS NULL
      AND closed_on IS NOT NULL
      AND data_source IN ('insight_github', 'insight_gitlab')
    SETTINGS mutations_sync = 1;

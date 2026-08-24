-- Add default-branch membership to the git commit class contract.
--
-- The column records whether a commit was reachable from the repository's
-- default branch at sync time, which gold reads to split commit output by
-- whether the work landed.
--
-- A pre-existing table gets it from nowhere: the DDL snapshot is IF NOT
-- EXISTS, and on_schema_change=append_new_columns only widens the relation
-- on a run of the model itself — which the deploy never performs, because
-- the hook builds tag:gold and the class models are tag:silver. Gold reads
-- the column in the same run, so without this the deploy fails on
-- UNKNOWN_IDENTIFIER. The staging halves need no heal: the projections are
-- rebuilt with the column by the descriptor bump that introduced it.
--
-- The AFTER anchor puts it where the staging projections place it and the
-- positional insert requires; MODIFY converges an instance where an
-- out-of-band ALTER placed it elsewhere. Shape follows
-- 20260820000000_git-file-change-blob-oids.sql.
--
-- Existing rows read NULL until the class model is re-materialized from
-- staging; gold resolves NULL as "not on the default branch".
--
-- Idempotent: this channel has no ledger and re-runs on every deploy.
-- The class tables always exist here (placeholders precede migrations).
ALTER TABLE silver.class_git_commits
    ADD COLUMN IF NOT EXISTS is_default_branch Nullable(UInt8) AFTER branch;

ALTER TABLE silver.class_git_commits
    MODIFY COLUMN is_default_branch Nullable(UInt8) AFTER branch;

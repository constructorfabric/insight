-- Add the per-commit patch id to the git class contract.
--
-- The patch id is the content identity of a commit's diff: a rebase copy or a
-- cherry-pick carries a new hash but the same patch id, and readers
-- deduplicate on it so one authored change counts once however many lines of
-- history re-apply it.
--
-- A pre-existing table gets it from nowhere: the DDL snapshot is
-- IF NOT EXISTS, and this model runs with dbt's default
-- on_schema_change semantics for warm tables, so an existing relation never
-- gains a column the model started projecting. The staging halves of the same
-- contract heal in apply-ch-migrations.sh, where a table that no connector has
-- built yet can be skipped.
--
-- AFTER anchors put it at the tail, the position the staging projections use
-- and the positional insert requires; MODIFY converges an instance where an
-- out-of-band ALTER placed it elsewhere. Shape follows
-- 20260820000000_git-file-change-blob-oids.sql.
--
-- Idempotent: this channel has no ledger and re-runs on every deploy.
-- The class tables always exist here (placeholders precede migrations).
ALTER TABLE silver.class_git_commits
    ADD COLUMN IF NOT EXISTS patch_id Nullable(String) AFTER _airbyte_extracted_at;

ALTER TABLE silver.class_git_commits
    MODIFY COLUMN patch_id Nullable(String) AFTER _airbyte_extracted_at;

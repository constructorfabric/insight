-- Add the trunk branch name to the git repository class contract.
--
-- Every vendor reports it (GitHub `default_branch`, Bitbucket
-- `mainbranch.name`, GitLab `default_branch`) and every staging model now
-- projects it; without the column, trunk-scoped reads have to guess the
-- branch by name and silently drop any repository with a non-standard trunk.
--
-- A pre-existing table gets it from nowhere: the DDL snapshot is
-- IF NOT EXISTS, and these models run with dbt's default
-- on_schema_change=ignore, so an existing relation never gains a column the
-- model started projecting. The staging halves of the same contract heal in
-- apply-ch-migrations.sh, where a table that no connector has built yet can
-- be skipped.
--
-- The AFTER anchor puts it at the tail, the position the staging projections
-- use and the positional insert requires; MODIFY converges an instance where
-- an out-of-band ALTER placed it elsewhere. Shape follows
-- 20260820000000_git-file-change-blob-oids.sql.
--
-- Idempotent: this channel has no ledger and re-runs on every deploy.
-- The class tables always exist here (placeholders precede migrations).
ALTER TABLE silver.class_git_repositories
    ADD COLUMN IF NOT EXISTS default_branch Nullable(String) AFTER _airbyte_extracted_at;

ALTER TABLE silver.class_git_repositories
    MODIFY COLUMN default_branch Nullable(String) AFTER _airbyte_extracted_at;

-- Carry the committer date alongside the author date on the git class contract.
--
-- `date` now holds the AUTHOR date, because that is when the work was written
-- and a rebase does not move it (#3153). That same property makes the author
-- date useless for telling a rebase copy apart from its original: both carry
-- it unchanged. Two readers need exactly that discrimination —
-- `git_derived_commits` ranks a patch id's carriers to pick the one that
-- authored it, and the file-content dedup in `git_metric_evidence` picks the
-- row that survives — and both fall through to a lexicographic hash compare
-- when the dates tie. The committer date is what still differs.
--
-- `ci.commits_observed` needs it for a second reason: CI runs are dated by
-- when they ran, so its commit side has to be dated the same way to line up.
--
-- A pre-existing table gets it from nowhere: the DDL snapshot is
-- IF NOT EXISTS, and this model runs with dbt's default on_schema_change
-- semantics for warm tables, so an existing relation never gains a column the
-- model started projecting. The staging halves of the same contract heal in
-- apply-ch-migrations.sh, where a table that no connector has built yet can be
-- skipped.
--
-- AFTER anchors put it at the tail, the position the staging projections use
-- and the positional insert requires; MODIFY converges an instance where an
-- out-of-band ALTER placed it elsewhere. Shape follows
-- 20260825000000_git-commit-patch-id.sql.
--
-- Idempotent: this channel has no ledger and re-runs on every deploy.
-- The class tables always exist here (placeholders precede migrations).
ALTER TABLE silver.class_git_commits
    ADD COLUMN IF NOT EXISTS committer_date Nullable(DateTime) AFTER patch_id;

ALTER TABLE silver.class_git_commits
    MODIFY COLUMN committer_date Nullable(DateTime) AFTER patch_id;

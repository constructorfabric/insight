-- Add the file-change object ids to the git class contract.
--
-- The columns are the content identity of a change (pre-image and post-image
-- oid), which readers deduplicate on: the same content entering a repository
-- on two lines of history is two commits but one post-image oid.
--
-- A pre-existing table gets them from nowhere: the DDL snapshot is
-- IF NOT EXISTS, and these models run with dbt's default
-- on_schema_change=ignore, so an existing relation never gains a column the
-- model started projecting. The staging halves of the same contract heal in
-- apply-ch-migrations.sh, where a table that no connector has built yet can be
-- skipped.
--
-- AFTER anchors put them at the tail, the position the staging projections use
-- and the positional insert requires; MODIFY converges an instance where an
-- out-of-band ALTER placed them elsewhere. Shape follows
-- 20260818000000_ai-dev-usage-seat-status.sql.
--
-- Idempotent: this channel has no ledger and re-runs on every deploy.
-- The class tables always exist here (placeholders precede migrations).
ALTER TABLE silver.class_git_file_changes
    ADD COLUMN IF NOT EXISTS pre_image_oid Nullable(String) AFTER _airbyte_extracted_at;

ALTER TABLE silver.class_git_file_changes
    ADD COLUMN IF NOT EXISTS post_image_oid Nullable(String) AFTER pre_image_oid;

ALTER TABLE silver.class_git_file_changes
    MODIFY COLUMN pre_image_oid Nullable(String) AFTER _airbyte_extracted_at;

ALTER TABLE silver.class_git_file_changes
    MODIFY COLUMN post_image_oid Nullable(String) AFTER pre_image_oid;

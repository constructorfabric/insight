-- Heal a sync ledger created with `job_created_at`.
--
-- The column was named for a field the mover's job listing does not report:
-- entries carry a last-update stamp, not a creation time, so every job was
-- refused for having no readable creation time and the page showed every
-- connector as never synced. The sweep now reads and filters by the stamp the
-- listing does report, and the column carries that name.
--
-- `CREATE TABLE IF NOT EXISTS` never renames a column of a table that already
-- exists, and this channel keeps no ledger — it re-runs on every deploy — so
-- the rename is guarded and a second run is a no-op. Only sync rows ever
-- carried the column, so an install that has recorded none loses nothing; one
-- that has keeps its stamps under the truthful name.
--
-- Spec: docs/components/backend/analytics/specs/connector-health.

ALTER TABLE ingestion_history.sync_events
    RENAME COLUMN IF EXISTS job_created_at TO job_updated_at;

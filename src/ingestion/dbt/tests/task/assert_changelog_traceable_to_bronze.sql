-- Every changelog silver row must join back to a staging row by natural key.
-- Broken joins mean either a dbt bug upstream (dropped rows) or enrich wrote rows for a
-- changelog_id that doesn't exist in bronze — both indicate data corruption.
--
-- Jira-only for now; other sources get their own staging table checks.

SELECT
    fh.insight_source_id,
    fh.id_readable,
    fh.event_id,
    fh.field_id
FROM silver.class_task_field_history fh FINAL
-- staging.jira_changelog_items is plain MergeTree (append-only dbt incremental), no FINAL needed
LEFT JOIN staging.jira_changelog_items ci
    ON fh.insight_source_id = ci.insight_source_id
   AND fh.id_readable       = ci.id_readable
   AND fh.event_id          = ci.changelog_id
   AND fh.field_id          = ci.field_id
WHERE fh.data_source = 'jira'
  AND fh.event_kind  = 'changelog'
  AND ci.changelog_id IS NULL
LIMIT 100

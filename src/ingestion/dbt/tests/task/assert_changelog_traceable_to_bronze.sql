{{ config(
    tags=['connector_quality', 'jira'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'Every Jira changelog row in silver traces back to a changelog item',
        'domain': 'task-tracking',
        'category': 'traceability',
        'tier': 'error',
        'remediation': 'A changelog event in class_task_field_history has no matching item in staging.jira_changelog_items for its source, issue, changelog id and field. Either silver holds rows a retired producer wrote, or the journal derived an event from nothing; rebuild the journal and compare.'
    }
) }}

-- Every changelog silver row must join back to a staging row by natural key.
-- A broken join means a producer wrote a row for a changelog_id that does not
-- exist in bronze — data corruption, whichever model did it.
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
   AND fh.issue_id          = ci.jira_id
   AND fh.event_id          = ci.changelog_id
   AND fh.field_id          = ci.field_id
WHERE fh.data_source = 'jira'
  AND fh.event_kind  = 'changelog'
  AND ci.changelog_id IS NULL

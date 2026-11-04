{{ config(
    tags=['connector_quality', 'jira'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'Every Jira initial-state row in silver has its issue in bronze',
        'domain': 'task-tracking',
        'category': 'traceability',
        'tier': 'warn',
        'remediation': 'A synthetic initial row in class_task_field_history names a Jira issue bronze does not hold. After bronze is cleared or a source is recreated, silver keeps the old rows until the class is fully refreshed; otherwise the row is an orphan of a retired producer.'
    }
) }}

-- Every initial silver row must correspond to an issue row in bronze_jira.jira_issue
-- (for Jira source). Convention: event_id = 'initial:<issue_id>' where issue_id = jira_id.

SELECT
    fh.insight_source_id,
    fh.id_readable,
    fh.issue_id,
    fh.event_id
FROM silver.class_task_field_history fh FINAL
-- INVARIANT: `matched` marks a joined row under either join_use_nulls setting.
LEFT JOIN (
    SELECT source_id, toString(jira_id) AS jira_id, 1 AS matched
    FROM {{ source('bronze_jira', 'jira_issue') }}
) i
    ON fh.insight_source_id = i.source_id
   AND fh.issue_id          = i.jira_id
WHERE fh.data_source = 'jira'
  AND fh.event_kind  = 'synthetic_initial'
  AND ifNull(i.matched, 0) = 0

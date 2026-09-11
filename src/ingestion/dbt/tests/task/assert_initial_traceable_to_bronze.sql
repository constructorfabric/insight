-- Every initial silver row must correspond to an issue row in bronze_jira.jira_issue
-- (for Jira source). Convention: event_id = 'initial:<issue_id>' where issue_id = jira_id.

SELECT
    fh.insight_source_id,
    fh.id_readable,
    fh.issue_id,
    fh.event_id
FROM silver.class_task_field_history fh FINAL
LEFT JOIN {{ source('bronze_jira', 'jira_issue') }} i
    ON fh.insight_source_id = i.source_id
   AND fh.issue_id          = toString(i.jira_id)
WHERE fh.data_source = 'jira'
  AND fh.event_kind  = 'synthetic_initial'
  AND i.source_id IS NULL
LIMIT 100

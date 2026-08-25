-- API-to-Bronze completeness (#2419): a worklog with an authoritative
-- /worklog/deleted tombstone must be flagged is_deleted in the class contract.
-- A violation means the staging model's deletion signals regressed and gold
-- metrics are counting time logged on worklogs Jira no longer has.

SELECT
    w.insight_source_id,
    w.worklog_id
FROM silver.class_task_worklogs AS w FINAL
INNER JOIN bronze_jira.jira_worklog_deleted AS t FINAL
    ON t.source_id = w.insight_source_id
    AND toString(toInt64(t.worklog_id)) = w.worklog_id
WHERE w.data_source = 'jira'
  AND ifNull(w.is_deleted, 0) = 0

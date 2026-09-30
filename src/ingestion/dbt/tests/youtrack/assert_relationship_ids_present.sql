SELECT unique_key, 'comment' AS entity
FROM {{ ref('class_task_comments') }} FINAL
WHERE data_source = 'youtrack' AND nullIf(issue_id, '') IS NULL

UNION ALL

SELECT unique_key, 'worklog' AS entity
FROM {{ ref('class_task_worklogs') }} FINAL
WHERE data_source = 'youtrack' AND nullIf(issue_id, '') IS NULL

UNION ALL

SELECT unique_key, 'link' AS entity
FROM {{ ref('class_task_links') }} FINAL
WHERE data_source = 'youtrack'
  AND (nullIf(issue_id, '') IS NULL OR nullIf(target_id, '') IS NULL)

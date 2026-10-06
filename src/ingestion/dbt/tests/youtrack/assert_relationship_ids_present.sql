{{ config(
    tags=['connector_quality', 'youtrack'],
    meta={
        'title': 'Task comments, worklogs and links name their issue and target',
        'domain': 'task-tracking',
        'category': 'integrity',
        'tier': 'error',
        'remediation': 'A silver row lost the key that ties it to an issue. Check the youtrack__task_* producer of that entity for a join or extraction that yields an empty id.'
    }
) }}

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

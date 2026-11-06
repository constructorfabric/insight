{{ config(
    tags=['connector_quality', 'jira'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'Jira field history rows in silver carry the issue-keyed unique key',
        'domain': 'task-tracking',
        'category': 'integrity',
        'tier': 'warn',
        'remediation': 'A Jira row of class_task_field_history has an empty issue_id or a unique_key that jira_history_key does not produce. Rows written by an older key formula stay until the class is fully refreshed; a new one means the formula drifted.'
    }
) }}

-- Every Jira row of the class carries the `unique_key` that `jira_history_key`
-- states, with the issue named by its immutable id. The staging twin
-- (`assert_jira_field_history_key_is_issue_keyed`) checks the derived journal
-- alone; this one checks what actually reached silver, so a row written by a
-- producer that is no longer a model — or by a formula that drifted — is seen
-- where gold reads it. An empty `issue_id` fails it too.

SELECT unique_key, issue_id, id_readable, field_id, event_id
FROM {{ ref('class_task_field_history') }} FINAL
WHERE data_source = 'jira'
  AND (issue_id = ''
       OR unique_key != {{ jira_history_key('insight_source_id', 'issue_id', 'field_id', 'event_id') }})

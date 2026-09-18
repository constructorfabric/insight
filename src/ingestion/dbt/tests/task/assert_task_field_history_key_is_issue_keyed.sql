{{ config(tags=['jira', 'silver']) }}

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
LIMIT 100

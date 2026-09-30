SELECT h.insight_source_id, h.issue_id, h.field_id, h.event_id
FROM {{ ref('youtrack__task_field_history') }} AS h FINAL
INNER JOIN {{ ref('task_field_roles_current') }} AS r
    ON r.insight_source_id = h.insight_source_id AND r.data_source = h.data_source AND r.field_id = h.field_id
WHERE r.role IN ('status', 'assignee', 'issuetype', 'resolution', 'estimate', 'spent', 'duedate')
  AND length(h.value_ids) > 1

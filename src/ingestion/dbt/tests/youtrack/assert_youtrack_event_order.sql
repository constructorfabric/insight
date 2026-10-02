SELECT insight_source_id, issue_id, field_id, event_order
FROM {{ ref('youtrack__task_field_history') }} FINAL
GROUP BY insight_source_id, issue_id, field_id, event_order
HAVING count() > 1

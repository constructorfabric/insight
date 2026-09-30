SELECT insight_source_id, issue_id, field_id, event_id
FROM {{ ref('youtrack__task_field_history') }} FINAL
WHERE length(value_ids) != length(value_displays)
   OR length(value_ids) != length(arrayDistinct(value_ids))
   OR (field_cardinality = 'single' AND length(value_ids) > 1)

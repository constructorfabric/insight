{{ config(
    tags=['connector_quality', 'youtrack'],
    meta={
        'title': 'Field history value arrays are aligned and distinct',
        'domain': 'task-tracking',
        'category': 'integrity',
        'tier': 'error',
        'remediation': 'value_ids and value_displays disagree in length, repeat an id, or a single field holds several values. Check youtrack__field_states for the listed events.'
    }
) }}

SELECT insight_source_id, issue_id, field_id, event_id
FROM {{ ref('youtrack__task_field_history') }} FINAL
WHERE length(value_ids) != length(value_displays)
   OR length(value_ids) != length(arrayDistinct(value_ids))
   OR (field_cardinality = 'single' AND length(value_ids) > 1)

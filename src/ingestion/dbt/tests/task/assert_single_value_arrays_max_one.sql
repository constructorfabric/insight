{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'A single-value field holds at most one value',
        'domain': 'task-tracking',
        'category': 'integrity',
        'tier': 'warn',
        'remediation': 'A field declared single-valued carries more than one value id in one row. Check the producer\'s cardinality for the field named in the finding.'
    }
) }}

-- Single-value field rows must have at most one element in value_ids / value_displays.

SELECT
    insight_source_id,
    data_source,
    issue_id,
    field_id,
    event_id,
    length(value_ids) AS n
FROM silver.class_task_field_history FINAL
WHERE field_cardinality = 'single'
  AND length(value_ids) > 1

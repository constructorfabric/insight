{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'A field value holds no duplicate ids',
        'domain': 'task-tracking',
        'category': 'integrity',
        'tier': 'warn',
        'remediation': 'One field history row lists the same value id twice. The producer named by data_source repeats elements when it builds the value array.'
    }
) }}

-- value_ids must not contain duplicates within the same row.
-- A duplicate indicates either a buggy Add for a value already present, or a misapplied
-- Snapshot that preserved duplicates.

SELECT
    tenant_id,
    insight_source_id,
    data_source,
    issue_id,
    field_id,
    event_id,
    value_ids,
    length(value_ids)              AS n,
    length(arrayDistinct(value_ids)) AS n_distinct
FROM silver.class_task_field_history FINAL
WHERE length(value_ids) != length(arrayDistinct(value_ids))

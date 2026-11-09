{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'Field value ids and displays line up',
        'domain': 'task-tracking',
        'category': 'integrity',
        'tier': 'warn',
        'remediation': 'A field history row has a different number of value ids and value displays, which breaks every reader that zips them. The producer named by data_source builds the two arrays separately; fix it to emit them together.'
    }
) }}

-- Every row must have parallel arrays — same number of ids and displays.
-- Failures indicate a bug in `core::emit_*_row` or snapshot accumulator.

SELECT
    tenant_id,
    insight_source_id,
    data_source,
    issue_id,
    field_id,
    event_id,
    length(value_ids)      AS n_ids,
    length(value_displays) AS n_displays
FROM silver.class_task_field_history FINAL
WHERE length(value_ids) != length(value_displays)

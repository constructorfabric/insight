{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'An issue field\'s initial state precedes its first change',
        'domain': 'task-tracking',
        'category': 'consistency',
        'tier': 'warn',
        'remediation': 'A synthetic initial row orders after the first changelog event of its field, so the history starts from the wrong value. The producer named by data_source ordered its initial row incorrectly.'
    }
) }}

-- The state at creation is by definition the state before every event, even
-- when an imported history dates an event before the issue's own creation.

SELECT
    tenant_id,
    insight_source_id,
    data_source,
    issue_id,
    maxIf(event_order, event_kind = 'synthetic_initial') AS last_initial_order,
    minIf(event_order, event_kind = 'changelog')         AS first_event_order
FROM silver.class_task_field_history FINAL
GROUP BY tenant_id, insight_source_id, data_source, issue_id
HAVING countIf(event_kind = 'synthetic_initial') > 0
   AND countIf(event_kind = 'changelog') > 0
   AND last_initial_order >= first_event_order

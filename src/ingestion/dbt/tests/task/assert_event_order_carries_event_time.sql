{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'Field history event order starts with the event time',
        'domain': 'task-tracking',
        'category': 'consistency',
        'tier': 'error',
        'remediation': 'An event_order does not begin with the millisecond of its event_at, so ordering by event_order disagrees with time. The producer did not compute the order through the shared task_event_order macro.'
    }
) }}

-- `event_order` leads with the event's millisecond on every row except
-- `synthetic_initial`, which orders at the earlier of the issue's creation and
-- its first event (`task_event_order`). A row whose order drifts from its time
-- sorts against the wrong neighbours.

SELECT
    insight_source_id,
    data_source,
    issue_id,
    field_id,
    event_id,
    event_kind,
    event_at,
    event_order
FROM silver.class_task_field_history FINAL
WHERE event_kind != 'synthetic_initial'
  AND event_order - toUnixTimestamp64Milli(event_at) * 1000000 NOT BETWEEN 0 AND 999999

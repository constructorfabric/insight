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
LIMIT 100

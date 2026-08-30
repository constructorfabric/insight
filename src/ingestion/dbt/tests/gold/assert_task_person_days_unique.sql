-- One row per person and day: in-progress time and logged time are summed into
-- that grain, never repeated across it.
SELECT
    tenant_id,
    entity_id,
    metric_date,
    count() AS row_count
FROM {{ ref('task_worklog_flow') }}
GROUP BY tenant_id, entity_id, metric_date
HAVING count() > 1

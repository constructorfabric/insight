-- One row per person and closing day: the day is the unit the estimate ratio
-- is taken over, so it may not appear twice.
SELECT
    tenant_id,
    assignee_email,
    closed_date,
    count() AS row_count
FROM {{ ref('task_estimation_days') }}
GROUP BY tenant_id, assignee_email, closed_date
HAVING count() > 1

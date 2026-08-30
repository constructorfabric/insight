-- One reading per seat per day: the step between readings is only a day's spend
-- while the day holds a single one.
SELECT
    tenant_id,
    source_id,
    account_id,
    snapshot_date,
    count() AS row_count
FROM {{ ref('ai_seat_days') }}
GROUP BY tenant_id, source_id, account_id, snapshot_date
HAVING count() > 1

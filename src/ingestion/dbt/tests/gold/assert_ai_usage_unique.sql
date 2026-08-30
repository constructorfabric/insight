-- One row per person, day, tool, surface and seat state: the counters of every
-- connector instance reporting that combination are summed into it, never
-- repeated across it.
SELECT
    tenant_id,
    email,
    usage_date,
    tool,
    surface,
    seat_status,
    count() AS row_count
FROM {{ ref('ai_usage') }}
GROUP BY tenant_id, email, usage_date, tool, surface, seat_status
HAVING count() > 1

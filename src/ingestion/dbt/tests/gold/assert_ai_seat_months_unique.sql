-- A seat is unique only within the connector instance that reported it, and it
-- is priced once per billing month.
SELECT
    tenant_id,
    source_id,
    account_id,
    period_month,
    count() AS row_count
FROM {{ ref('ai_seat_months') }}
GROUP BY tenant_id, source_id, account_id, period_month
HAVING count() > 1

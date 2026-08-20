{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'Active Claude Team seats report an allowance',
        'domain': 'ai',
        'category': 'coverage',
        'tier': 'error',
        'remediation': 'A person cannot use Claude Team without occupying a seat, so activity in class_ai_dev_usage implies a class_ai_overage row for the same billing month. Rows here mean the overage stream is not reporting seats that are demonstrably in use. First check the scope on the session key the connector uses: /api/organizations/{org}/overage_spend_limits answers 403 without billing:view, and the connector maps 403 to action: IGNORE, so the sync stays green and the stream silently yields nothing. If the permission is intact, compare the seat set the endpoint returns against the emails listed here — a seat removed mid-month is a legitimate explanation for an individual row, an empty result is not.'
    }
) }}
{#- Bounded to the current billing month on purpose. overage_spend_limits is a
    snapshot of the month in progress and is never backfilled, so a completed
    month only holds the seats that happened to be captured while it was
    current: months that predate the connector carry activity with no seat row,
    and nothing can fill them in later. Unbounded, this check would report that
    gap forever. -#}

WITH toStartOfMonth(today()) AS billing_month,

active_seats AS (

    {#- Activity from yesterday onward is legitimately mid-flight: a seat can
        start producing usage before the next daily snapshot lands, and on the
        first days of a month that would flag every seat at once. -#}
    SELECT DISTINCT
        insight_tenant_id,
        source_id,
        email
    FROM {{ ref('class_ai_dev_usage') }} FINAL
    WHERE source = 'claude_team'
      AND toStartOfMonth(day) = billing_month
      AND day < today() - 1

),

reported_seats AS (

    SELECT DISTINCT
        insight_tenant_id,
        source_id,
        email
    FROM {{ ref('class_ai_overage') }} FINAL
    WHERE source = 'claude_team'
      AND period_month = billing_month

)

SELECT
    a.insight_tenant_id AS insight_tenant_id,
    a.source_id         AS source_id,
    a.email             AS email,
    billing_month       AS period_month
FROM active_seats AS a
LEFT ANTI JOIN reported_seats AS r
    ON a.insight_tenant_id = r.insight_tenant_id
   AND a.source_id = r.source_id
   AND a.email = r.email

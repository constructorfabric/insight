{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'A seat source that reported last month still reports this month',
        'domain': 'ai',
        'category': 'coverage',
        'tier': 'error',
        'remediation': 'A row here is a connector instance that reported seats in the previous billing month and none in the current one. The seat endpoint returns the month in progress, so a healthy source reports every month it is installed for. The known cause is authorisation: /api/organizations/{org}/overage_spend_limits answers 403 once the session key the connector uses loses billing:view, and the connector maps 403 to action: IGNORE, so the sync stays green while the stream yields nothing. Restore the scope on the session key the connector uses, then re-sync. A source deliberately decommissioned mid-month is the other explanation, and it clears itself once the previous month rolls out of the comparison.'
    }
) }}
{#- The sibling check, assert_ai_overage_covers_active_seats, only sees a seat
    that also produced Claude Code activity in the same month, so a silent
    stream whose seats happen to be idle passes it unnoticed. This one compares
    the source against its own past instead of against activity, which is what
    catches the stream going quiet as a whole.

    Inert for the first two days of a billing month: the current month has no
    rows until that month's first sync lands, and flagging every source at
    midnight on the 1st would train the reader to ignore this check. -#}

WITH toStartOfMonth(today()) AS billing_month,

reported_last_month AS (

    SELECT DISTINCT
        insight_tenant_id,
        source_id,
        source
    FROM {{ ref('class_ai_overage') }} FINAL
    WHERE period_month = billing_month - INTERVAL 1 MONTH

),

reporting_this_month AS (

    SELECT DISTINCT
        insight_tenant_id,
        source_id,
        source
    FROM {{ ref('class_ai_overage') }} FINAL
    WHERE period_month = billing_month

)

SELECT
    p.insight_tenant_id AS insight_tenant_id,
    p.source_id         AS source_id,
    p.source            AS source,
    billing_month       AS period_month
FROM reported_last_month AS p
LEFT ANTI JOIN reporting_this_month AS c
    ON p.insight_tenant_id = c.insight_tenant_id
   AND p.source_id = c.source_id
   AND p.source = c.source
WHERE today() >= billing_month + 2

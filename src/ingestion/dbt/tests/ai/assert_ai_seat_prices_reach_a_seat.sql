{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'An invoiced seat price reaches a seat',
        'domain': 'ai',
        'category': 'coverage',
        'tier': 'error',
        'remediation': 'A row here is a month whose vendor invoice priced a seat while some seat got no fee, so that money sits on the ledger and ai.seat_cost serves nothing from it. The finding column says which of the two states it is, and they are fixed in different places. no_tiered_seats: the invoice priced seats and the month holds none that a fee can reach, so no binding can help — the seat connector has not delivered, and its last sync is what to check. tier_unpriced: seats of this tier exist and took no fee, so the binding is what is missing or wrong; add or correct a row in config.ai_seat_tier_map, which dbt does not own, naming tenant_id, the invoice connector instance in insight_source_id, the class source, the vendor catalogue identifier in tier_ref, the seat_tier those seats carry, and — where the tenant runs two connector instances on either side — the seat population in seat_source_id. In neither state is the transformation at fault: gold declines to price a seat it cannot attribute rather than guessing. A month whose only priced tier is unambiguous needs no binding at all, and for such an installation an empty map is the correct state.'
    }
) }}
{#- Reports two observable states and deliberately does NOT reproduce the
    reachability rule gold applies (`ai_cost_metric_evidence.sql`: offers filtered
    per seat population, then a tier disambiguation). It reads that model's
    OUTPUT instead, so it cannot drift from the rule that decided it — a copy of
    that rule lived here once and was wrong in both directions.

    What the outcome cannot see, and the trade taken knowingly: a priced tier
    that reaches nobody while every seat that does exist is priced — a stale
    binding for a tier the organisation no longer holds. Nobody is missing a
    figure in that state, which is why it is the half worth giving up for a check
    that cannot silently disagree with the model.

    Bounded to the last three billing months: a tier retired long ago has no seat
    population left to reach and would report forever. Recent months are the ones
    an operator can still act on. -#}

WITH
-- The only invoice shape that states a per-seat amount, so the only one whose
-- absence downstream is a coverage gap rather than a shape the vendor never sent.
priced_months AS (
    SELECT DISTINCT
        invoice.insight_tenant_id               AS tenant_id,
        invoice.source                          AS source,
        invoice.period_month                    AS period_month
    FROM {{ ref('class_ai_invoice') }} AS invoice FINAL
    WHERE invoice.line_id IS NOT NULL
      AND invoice.category = 'subscriptions'
      AND invoice.is_proration = 0
      AND invoice.seat_unit_cents IS NOT NULL
      AND invoice.insight_tenant_id IS NOT NULL
      AND invoice.period_month >= toStartOfMonth(today()) - INTERVAL 2 MONTH
),

-- A seat a fee can reach at all. gold gates the seat measure on the ceiling, so
-- a seat without one is not a seat any invoice can price. `source` joins the
-- invoice side and `tool` joins the served side, which is why both are kept.
tiered_seats AS (
    SELECT
        insight_tenant_id                       AS tenant_id,
        source,
        tool,
        period_month,
        coalesce(seat_tier, '')                 AS seat_tier,
        count()                                 AS seats
    FROM {{ ref('class_ai_overage') }} FINAL
    WHERE credit_limit_cents IS NOT NULL
      AND email IS NOT NULL
      AND email != ''
      AND insight_tenant_id IS NOT NULL
    GROUP BY tenant_id, source, tool, period_month, seat_tier
),

served_tiers AS (
    SELECT DISTINCT
        tenant_id,
        arrayFirst(d -> d.1 = 'tool', dimensions).2      AS tool,
        metric_date                                      AS period_month,
        arrayFirst(d -> d.1 = 'seat_tier', dimensions).2 AS seat_tier
    FROM {{ ref('ai_cost_metric_evidence') }}
    WHERE measure_key = 'seat_cost_usd'
),

-- The invoice priced a seat and the month holds none to price.
no_tiered_seats AS (
    SELECT
        tenant_id,
        source,
        period_month,
        'no_tiered_seats'                       AS finding,
        ''                                      AS seat_tier,
        toUInt64(0)                             AS seats
    FROM priced_months
    -- INVARIANT: a tuple NOT IN, never a LEFT JOIN tested for NULL. An unmatched
    -- ClickHouse left join yields the column's default, which for a count is 0
    -- and would read as a real answer.
    WHERE (tenant_id, source, period_month) NOT IN (
            SELECT tenant_id, source, period_month FROM tiered_seats
        )
),

-- Seats of this tier exist, the month priced a seat, and gold served none of
-- them a fee.
tier_unpriced AS (
    SELECT
        tenant_id,
        source,
        period_month,
        'tier_unpriced'                         AS finding,
        seat_tier,
        seats
    FROM tiered_seats
    WHERE (tenant_id, source, period_month) IN (
            SELECT tenant_id, source, period_month FROM priced_months
        )
      AND (tenant_id, tool, period_month, seat_tier) NOT IN (
            SELECT tenant_id, tool, period_month, seat_tier FROM served_tiers
        )
)

SELECT tenant_id, source, period_month, finding, seat_tier, seats
FROM no_tiered_seats
UNION ALL
SELECT tenant_id, source, period_month, finding, seat_tier, seats
FROM tier_unpriced

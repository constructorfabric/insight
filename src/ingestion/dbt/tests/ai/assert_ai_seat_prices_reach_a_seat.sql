{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'An invoiced seat price reaches a seat',
        'domain': 'ai',
        'category': 'coverage',
        'tier': 'error',
        'remediation': 'A row here is a priced seat tier on a vendor invoice that no seat in that month can be matched to, so ai.seat_cost serves nothing for those people while the money sits on the ledger. The binding is per installation and dbt does not own it: add a row to config.ai_seat_tier_map naming tenant_id, the invoice connector instance in insight_source_id, the class source, the vendor catalogue identifier in tier_ref, and the seat_tier a seat carries in silver.class_ai_overage. seat_tier is reported empty when nothing binds tier_ref at all, and non-empty when a binding exists but names a tier no seat carries — a stale or mistyped tier. A tenant running two instances of the vendor on either side must also name the seat population in seat_source_id. Nothing is wrong with the pipeline: gold refuses to price a seat from an ambiguous month rather than guessing which tier an amount belongs to.'
    }
) }}
{#- A month carrying exactly one priced offer is not reported: gold prices every
    tiered seat from an unambiguous month, so a single-tier installation needs no
    binding at all and an empty map is its correct state.

    Bounded to the last three billing months for the same reason as its sibling:
    a tier retired long ago has no seat population left to reach and would report
    forever. Recent months are the ones a binding can still fix. -#}

WITH priced_offers AS (
    SELECT
        invoice.insight_tenant_id               AS tenant_id,
        invoice.source                          AS source,
        invoice.period_month                    AS period_month,
        invoice.tier_ref                        AS tier_ref,
        coalesce(binding.seat_tier, '')         AS seat_tier,
        invoice.seat_unit_cents                 AS seat_unit_cents
    FROM {{ ref('class_ai_invoice') }} AS invoice FINAL
    LEFT JOIN (
        SELECT
            tenant_id,
            insight_source_id,
            source,
            tier_ref,
            seat_tier
        FROM {{ source('config', 'ai_seat_tier_map') }} FINAL
        WHERE is_deleted = 0
    ) AS binding
        ON  binding.tenant_id = invoice.insight_tenant_id
        AND binding.insight_source_id = invoice.source_id
        AND binding.source = invoice.source
        AND binding.tier_ref = invoice.tier_ref
    WHERE invoice.line_id IS NOT NULL
      AND invoice.category = 'subscriptions'
      AND invoice.is_proration = 0
      AND invoice.seat_unit_cents IS NOT NULL
      AND invoice.period_month >= toStartOfMonth(today()) - INTERVAL 2 MONTH
    GROUP BY tenant_id, source, period_month, tier_ref, seat_tier, seat_unit_cents
),

ambiguous_months AS (
    SELECT
        tenant_id,
        source,
        period_month
    FROM priced_offers
    GROUP BY tenant_id, source, period_month
    HAVING count() > 1
),

seat_tiers AS (
    SELECT
        insight_tenant_id                       AS tenant_id,
        source,
        period_month,
        coalesce(seat_tier, '')                 AS seat_tier
    FROM {{ ref('class_ai_overage') }} FINAL
    WHERE credit_limit_cents IS NOT NULL
    GROUP BY tenant_id, source, period_month, seat_tier
)

SELECT
    tenant_id,
    source,
    period_month,
    tier_ref,
    seat_tier,
    seat_unit_cents
FROM priced_offers
WHERE (tenant_id, source, period_month) IN (
        SELECT tenant_id, source, period_month FROM ambiguous_months
    )
  -- INVARIANT: a tuple NOT IN, never a LEFT JOIN tested for NULL. An unmatched
  -- ClickHouse left join yields the column's default, so an empty seat_tier
  -- would read as a match rather than as the miss it is.
  AND (tenant_id, source, period_month, seat_tier) NOT IN (
        SELECT tenant_id, source, period_month, seat_tier FROM seat_tiers
    )

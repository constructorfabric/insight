{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'email', 'period_month'],
    partition_by='toYYYYMM(period_month)',
    schema=var('gold_database'),
    settings={'allow_nullable_key': 1},
    tags=['gold'],
    query_settings=metric_serving_query_settings()
) }}

-- What one AI seat cost in one billing month: the fee the invoice priced it at,
-- the usage billed on top of that fee, and the ceiling that usage runs against.
-- A semantic-layer dataset — measures sum it and a ratio reads the ceiling.

WITH
-- What the vendor charged for one seat in a billing month. Only a non-proration
-- `subscriptions` line states a per-seat amount: the invoice total covers every
-- seat and every proration together, and `num_seats` reports one line's quantity
-- while an invoice can price several tiers. Collected per tier so a month with
-- more than one priced tier stays distinguishable from an unambiguous one.
--
-- Each price carries the seat population it may reach. One tenant can hold
-- several instances of a vendor's connector, and the invoice instance and the
-- seat instance are separate connectors whose source_id never matches — so the
-- only thing that can say which seats an invoice billed is the operator's
-- binding. `invoice_sources` counts the instances that priced anything, which is
-- what tells an unscoped price whether it is the tenant's only one.
month_seat_prices AS (
    SELECT
        tenant_id,
        source,
        period_month,
        groupUniqArray(tuple(seat_source_id, tier, price)) AS scoped_prices,
        uniqExact(invoice_source_id)            AS invoice_sources
    FROM (
        SELECT
            invoice.insight_tenant_id           AS tenant_id,
            invoice.source                      AS source,
            invoice.period_month                AS period_month,
            invoice.source_id                   AS invoice_source_id,
            -- The seat connector instance this price applies to. Empty when the
            -- binding names none, which only a single-instance tenant can use.
            coalesce(binding.seat_source_id, '') AS seat_source_id,
            -- What a seat would call this line's tier, per the operator's
            -- binding. Unbound leaves it empty, and no seat tier is empty, so an
            -- unrecognised plan prices nothing rather than pricing a guess.
            coalesce(binding.seat_tier, '')      AS tier,
            invoice.seat_unit_cents             AS price
        FROM {{ ref('class_ai_invoice') }} AS invoice FINAL
        LEFT JOIN (
            SELECT
                tenant_id,
                insight_source_id,
                source,
                tier_ref,
                seat_source_id,
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
        GROUP BY tenant_id, source, period_month, invoice_source_id, seat_source_id, tier, price
    )
    GROUP BY tenant_id, source, period_month
),
seat_month_source AS (
    SELECT
        insight_tenant_id                       AS tenant_id,
        source,
        source_id,
        account_id,
        {{ normalized_email('email') }}         AS email,
        tool,
        seat_tier,
        period_month,
        used_amount_cents,
        credit_limit_cents
    FROM {{ ref('class_ai_overage') }} FINAL
    WHERE insight_tenant_id IS NOT NULL
      AND email IS NOT NULL
      AND email != ''
      AND collected_at IS NOT NULL
),
-- How many seat populations the tenant holds for this vendor in this month. An
-- unscoped price may only be taken when there is exactly one, because with two
-- nothing says which of them the invoice billed — not even when only one of them
-- produced an invoice at all.
month_seat_populations AS (
    SELECT
        tenant_id,
        source,
        period_month,
        uniqExact(source_id)                    AS seat_sources
    FROM seat_month_source
    GROUP BY tenant_id, source, period_month
),
-- How many populations the seat's own month holds, carried onto the seat.
--
-- INVARIANT: one join per level. Two joins in one SELECT beside `seat.*`, where
-- both joined relations carry a `tenant_id` of their own, leave the unqualified
-- name unresolvable and the model fails to build.
seat_month_scoped AS (
    SELECT
        seat.*,
        populations.seat_sources                AS seat_sources
    FROM seat_month_source AS seat
    LEFT JOIN month_seat_populations AS populations
        ON  populations.tenant_id = seat.tenant_id
        AND populations.source = seat.source
        AND populations.period_month = seat.period_month
),
-- The prices that can reach this seat: those the operator bound to the seat's
-- own connector instance, plus the unscoped ones when the tenant runs a single
-- instance on both sides. A tenant running two therefore never lends one
-- instance's price to the other's seats, whichever of them the invoice came from.
seat_month_offers AS (
    SELECT
        scoped.* EXCEPT (seat_sources),
        arrayFilter(
            p -> p.1 = scoped.source_id
                 OR (p.1 = '' AND prices.invoice_sources = 1 AND scoped.seat_sources = 1),
            prices.scoped_prices
        )                                       AS offers
    FROM seat_month_scoped AS scoped
    LEFT JOIN month_seat_prices AS prices
        ON  prices.tenant_id = scoped.tenant_id
        AND prices.source = scoped.source
        AND prices.period_month = scoped.period_month
),
-- The price this seat was billed at. One priced tier among the offers prices
-- every seat billed in it, which is the common shape and the only one the vendor
-- states unambiguously. With several, the seat's own tier has to name one of
-- them, which needs the operator's binding above: no vendor states that an
-- invoice line's tier and a seat's tier are the same tier. A seat that names
-- none is left without a price rather than given a share of the invoice total.
seat_month_priced AS (
    SELECT
        * EXCEPT (offers),
        multiIf(
            length(offers) = 1,
            offers[1].3,
            length(arrayFilter(p -> p.2 = coalesce(seat_tier, ''), offers)) = 1,
            arrayFilter(p -> p.2 = coalesce(seat_tier, ''), offers)[1].3,
            CAST(NULL AS Nullable(Int64))
        )                                       AS seat_price_cents
    FROM seat_month_offers
)

SELECT
    -- SAFETY: both are safe under the WHERE in `seat_month_source`.
    assumeNotNull(tenant_id)                    AS tenant_id,
    source_id                                   AS source_id,
    account_id                                  AS account_id,
    assumeNotNull(email)                        AS email,
    period_month                                AS period_month,
    tool                                        AS tool,
    {{ ai_tool_label('tool') }}                 AS tool_label,
    coalesce(seat_tier, 'unknown')              AS seat_tier,
    -- The money: what the vendor billed once the seat exhausted the usage
    -- included in its fee. NOT the excess over the ceiling below — that
    -- difference is where spending stopped, not what it cost.
    toFloat64(used_amount_cents) / 100          AS extra_usage_usd,
    -- Honest-NULL: a seat with no ceiling carries no denominator, so the ratio
    -- metric has no value for it rather than a fabricated one.
    if(
        credit_limit_cents IS NOT NULL,
        toNullable(toFloat64(credit_limit_cents) / 100),
        CAST(NULL AS Nullable(Float64))
    )                                           AS extra_usage_limit_usd,
    -- The seat fee itself, which the extra usage above sits on top of. Gated on
    -- the ceiling because that is what marks a seat as carrying a tier, and an
    -- untiered seat is not one the invoice billed for.
    if(
        seat_price_cents IS NOT NULL AND credit_limit_cents IS NOT NULL,
        toNullable(toFloat64(seat_price_cents) / 100),
        CAST(NULL AS Nullable(Float64))
    )                                           AS seat_cost_usd
FROM seat_month_priced

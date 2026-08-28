{{ metric_evidence_table() }}

-- Keyed by the source identity through `normalized_email()`, not by person:
-- the analytics runtime resolves through `identity.person_map` while it serves.
-- An unresolvable row stays and starts counting the moment it resolves.
SELECT
    src.tenant_id,
    src.source_key,
    src.entity_type,
    {{ normalized_email('src.entity_id') }} AS entity_id,
    -- No account-keyed facts here; '' leaves the account join unmatched.
    '' AS account_source_type,
    '' AS account_source_id,
    '' AS account_id,
    src.metric_date,
    src.observed_at,
    src.measure_key,
    src.record_id,
    src.record_kind,
    src.granularity,
    src.record_label,
    src.contribution,
    src.subject_key,
    src.dimensions,
    src.details
FROM (

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
        lower(email)                            AS entity_id,
        seat_tier,
        -- INVARIANT: month-anchored, not read-anchored — a read-day date moves
        -- with the sync schedule. `observed_at` keeps the time of the reading.
        toDate(period_month)                    AS metric_date,
        toDateTime64(collected_at, 3)           AS observed_at,
        period_month,
        CAST(
            [
                tuple('tool', tool, {{ ai_tool_label('tool') }}),
                tuple('seat_tier', coalesce(seat_tier, 'unknown'), CAST(NULL AS Nullable(String)))
            ] AS Array(Tuple(key String, value String, label Nullable(String)))
        )                                       AS seat_dimensions,
        used_amount_cents,
        credit_limit_cents
    FROM {{ ref('class_ai_overage') }} FINAL
    WHERE email IS NOT NULL
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
),
-- Every reading of a seat inside its billing month. The vendor reports a
-- cumulative month-to-date figure, so a day's spend is the step between two
-- readings and nothing else.
seat_day_source AS (
    SELECT
        insight_tenant_id                       AS tenant_id,
        source,
        source_id,
        account_id,
        lower(email)                            AS entity_id,
        snapshot_date                           AS metric_date,
        toDateTime64(collected_at, 3)           AS observed_at,
        period_month,
        CAST(
            [
                tuple('tool', tool, {{ ai_tool_label('tool') }}),
                tuple('seat_tier', coalesce(seat_tier, 'unknown'), CAST(NULL AS Nullable(String)))
            ] AS Array(Tuple(key String, value String, label Nullable(String)))
        )                                       AS seat_dimensions,
        used_amount_cents
    FROM {{ ref('class_ai_overage_daily') }} FINAL
    WHERE email IS NOT NULL
      AND email != ''
      AND snapshot_date IS NOT NULL
),
-- INVARIANT: a reading on the month's last calendar day may raise the month,
-- never lower it. No billing period is reported, so a drop there cannot be told
-- from a rolled-over counter, and the suffix minimum would spread it monthwide.
seat_day_held AS (
    SELECT
        *,
        -- INVARIANT: the PRECEDING reading, never the largest. The suffix
        -- minimum erases an earlier reading that was too high; a maximum over
        -- them would restore it.
        if(
            metric_date = toLastDayOfMonth(period_month),
            greatest(
                used_amount_cents,
                lagInFrame(used_amount_cents, 1, toUInt32(0)) OVER (
                    PARTITION BY tenant_id, source, source_id, account_id, period_month
                    ORDER BY metric_date
                )
            ),
            used_amount_cents
        )                                       AS held_cents
    FROM seat_day_source
),
-- INVARIANT: the suffix minimum is what keeps every step non-negative and makes
-- the steps add up to the month's final reading, which the monthly metric serves.
seat_day_corrected AS (
    SELECT
        *,
        min(held_cents) OVER (
            PARTITION BY tenant_id, source, source_id, account_id, period_month
            ORDER BY metric_date
            ROWS BETWEEN CURRENT ROW AND UNBOUNDED FOLLOWING
        )                                       AS corrected_cents
    FROM seat_day_held
),
seat_day_step AS (
    SELECT
        *,
        corrected_cents - lagInFrame(corrected_cents, 1, toUInt32(0)) OVER w
                                                AS step_cents,
        -- INVARIANT: covers_days spans from the previous reading, or from the 1st
        -- for a month's first one — above 1 the step is not one day's spend.
        toUInt16(dateDiff(
            'day',
            lagInFrame(metric_date, 1, toDate(period_month) - 1) OVER w,
            metric_date
        ))                                      AS covers_days
    FROM seat_day_corrected
    WINDOW w AS (
        PARTITION BY tenant_id, source, source_id, account_id, period_month
        ORDER BY metric_date
    )
)

SELECT
    assumeNotNull(tenant_id)                    AS tenant_id,
    'ai_cost'                                   AS source_key,
    'person'                                    AS entity_type,
    assumeNotNull(entity_id)                    AS entity_id,
    assumeNotNull(metric_date)                  AS metric_date,
    toNullable(observed_at)                     AS observed_at,
    seat_measure.1                              AS measure_key,
    -- Keyed on the billing month, not the read day: two months can in
    -- principle be read on one day at a month boundary, and both must survive.
    -- Connector instance and vendor seat id complete the silver grain, so two
    -- instances reporting one email stay two rows the drilldown cursor can
    -- order.
    concat(
        toString(period_month), ':', seat_measure.1, ':',
        hex(sipHash64(concat(coalesce(source_id, ''), ':', coalesce(account_id, ''))))
    )                                           AS record_id,
    'seat_month'                                AS record_kind,
    'source_summary'                            AS granularity,
    formatDateTime(period_month, '%Y-%m')       AS record_label,
    toNullable(toFloat64(seat_measure.2))       AS contribution,
    CAST(NULL AS Nullable(String))              AS subject_key,
    seat_dimensions                             AS dimensions,
    map(
        'billing_month', toString(period_month),
        'ceiling_usd', coalesce(toString(credit_limit_cents / 100), ''),
        'ceiling_set', if(credit_limit_cents IS NULL, 'false', 'true')
    )                                           AS details
FROM seat_month_priced
ARRAY JOIN arrayConcat(
    -- The money: what the vendor billed once the seat exhausted the usage
    -- included in its fee. NOT the excess over the ceiling below — that
    -- difference is where spending stopped, not what it cost.
    [tuple('extra_usage_usd', toFloat64(used_amount_cents) / 100)],
    -- Honest-NULL: a seat with no ceiling emits no row here, so the ratio
    -- metric has no denominator for it rather than a fabricated one.
    if(
        credit_limit_cents IS NOT NULL,
        [tuple('extra_usage_limit_usd', toFloat64(credit_limit_cents) / 100)],
        []
    ),
    -- The seat fee itself, which the extra usage above sits on top of. Gated on
    -- the ceiling because that is what marks a seat as carrying a tier, and an
    -- untiered seat is not one the invoice billed for.
    if(
        seat_price_cents IS NOT NULL AND credit_limit_cents IS NOT NULL,
        [tuple('seat_cost_usd', toFloat64(seat_price_cents) / 100)],
        []
    )
) AS seat_measure
WHERE tenant_id IS NOT NULL
  AND entity_id IS NOT NULL
  AND metric_date IS NOT NULL

UNION ALL

SELECT
    assumeNotNull(tenant_id)                    AS tenant_id,
    'ai_cost'                                   AS source_key,
    'person'                                    AS entity_type,
    assumeNotNull(entity_id)                    AS entity_id,
    assumeNotNull(metric_date)                  AS metric_date,
    toNullable(observed_at)                     AS observed_at,
    'daily_extra_usage_usd'                     AS measure_key,
    -- Keyed on the read day: unlike the month rows, the day IS the grain here.
    concat(
        toString(metric_date), ':daily_extra_usage_usd:',
        hex(sipHash64(concat(coalesce(source_id, ''), ':', coalesce(account_id, ''))))
    )                                           AS record_id,
    'seat_day'                                  AS record_kind,
    'source_summary'                            AS granularity,
    formatDateTime(metric_date, '%Y-%m-%d')     AS record_label,
    toNullable(toFloat64(step_cents) / 100)     AS contribution,
    CAST(NULL AS Nullable(String))              AS subject_key,
    seat_dimensions                             AS dimensions,
    map(
        'billing_month', toString(period_month),
        'month_to_date_usd', toString(toFloat64(corrected_cents) / 100),
        -- Above 1 the figure is a span, not a day: say so rather than let a
        -- reader take it for one day's spend.
        'covers_days', toString(covers_days)
    )                                           AS details
FROM seat_day_step
WHERE tenant_id IS NOT NULL
  AND entity_id IS NOT NULL
  AND metric_date IS NOT NULL
) AS src

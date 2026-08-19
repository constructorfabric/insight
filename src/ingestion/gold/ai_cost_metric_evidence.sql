{{ metric_evidence_table() }}

-- Resolution happens HERE, once per gold build: evidence carries BOTH keys —
-- `entity_id` is the canonical person id (or '' when identity does not know
-- the email: those rows stay for coverage but reach no serving relation), and
-- `source_entity_id` keeps the source-native email for provenance.
SELECT
    src.tenant_id,
    src.source_key,
    src.entity_type,
    if(
        coalesce(identity_map.email, '') != '',
        toString(assumeNotNull(identity_map.person_id)),
        ''
    ) AS entity_id,
    src.entity_id AS source_entity_id,
    src.metric_date,
    src.observed_at,
    src.measure_key,
    concat(src.record_id, ':', hex(sipHash64(src.entity_id))) AS record_id,
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
month_seat_prices AS (
    SELECT
        tenant_id,
        source,
        period_month,
        groupUniqArray(tuple(tier, price))      AS tier_prices
    FROM (
        SELECT
            insight_tenant_id                   AS tenant_id,
            source,
            period_month,
            coalesce(tier_label, '')            AS tier,
            seat_unit_cents                     AS price
        FROM {{ ref('class_ai_invoice') }} FINAL
        WHERE line_id IS NOT NULL
          AND category = 'subscriptions'
          AND is_proration = 0
          AND seat_unit_cents IS NOT NULL
        GROUP BY tenant_id, source, period_month, tier, price
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
        -- Dated at the day the snapshot was last read, NOT period_month. The
        -- vendor re-reads only the month in progress, so a month's row freezes
        -- at its final read; a date pinned to the 1st would fall outside short
        -- rolling windows and the current month would vanish from them.
        toDate(collected_at)                    AS metric_date,
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
-- The price this seat was billed at. One priced tier in the month prices every
-- seat billed in it, which is the common shape and the only one the vendor
-- states unambiguously. With several, the seat's own tier has to name one of
-- them: the two vocabularies come from different APIs and need not agree, so a
-- seat that names none is left without a price rather than given a share of the
-- invoice total.
seat_month_priced AS (
    SELECT
        seat.*,
        multiIf(
            length(prices.tier_prices) = 1,
            prices.tier_prices[1].2,
            length(arrayFilter(p -> p.1 = coalesce(seat.seat_tier, ''), prices.tier_prices)) = 1,
            arrayFilter(p -> p.1 = coalesce(seat.seat_tier, ''), prices.tier_prices)[1].2,
            CAST(NULL AS Nullable(Int64))
        )                                       AS seat_price_cents
    FROM seat_month_source AS seat
    LEFT JOIN month_seat_prices AS prices
        ON  prices.tenant_id = seat.tenant_id
        AND prices.source = seat.source
        AND prices.period_month = seat.period_month
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
) AS src
{{ resolved_person_id_join('src') }}

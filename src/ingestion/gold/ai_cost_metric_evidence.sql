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
seat_month_source AS (
    SELECT
        insight_tenant_id                       AS tenant_id,
        source_id,
        account_id,
        lower(email)                            AS entity_id,
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
FROM seat_month_source
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
    )
) AS seat_measure
WHERE tenant_id IS NOT NULL
  AND entity_id IS NOT NULL
  AND metric_date IS NOT NULL
) AS src
{{ resolved_person_id_join('src') }}

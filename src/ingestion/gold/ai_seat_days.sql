{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'email', 'snapshot_date'],
    partition_by='toYYYYMM(snapshot_date)',
    schema=var('gold_database'),
    settings={'allow_nullable_key': 1},
    tags=['gold'],
    query_settings=metric_serving_query_settings()
) }}

-- One AI seat's billed extra usage placed on the day it was spent. The vendor
-- reports a cumulative month-to-date figure, so a day's spend is the step
-- between two readings and nothing else — exact in sum over the month,
-- approximate in placement within it.

WITH
-- Every reading of a seat inside its billing month.
seat_day_source AS (
    SELECT
        insight_tenant_id                       AS tenant_id,
        source,
        source_id,
        account_id,
        {{ normalized_email('email') }}         AS email,
        tool,
        seat_tier,
        snapshot_date,
        period_month,
        used_amount_cents
    FROM {{ ref('class_ai_overage_daily') }} FINAL
    WHERE insight_tenant_id IS NOT NULL
      AND email IS NOT NULL
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
            snapshot_date = toLastDayOfMonth(period_month),
            greatest(
                used_amount_cents,
                lagInFrame(used_amount_cents, 1, toUInt32(0)) OVER (
                    PARTITION BY tenant_id, source, source_id, account_id, period_month
                    ORDER BY snapshot_date
                )
            ),
            used_amount_cents
        )                                       AS held_cents
    FROM seat_day_source
),
-- INVARIANT: the suffix minimum is what keeps every step non-negative and makes
-- the steps add up to the month's final reading, which the monthly dataset
-- serves.
seat_day_corrected AS (
    SELECT
        *,
        min(held_cents) OVER (
            PARTITION BY tenant_id, source, source_id, account_id, period_month
            ORDER BY snapshot_date
            ROWS BETWEEN CURRENT ROW AND UNBOUNDED FOLLOWING
        )                                       AS corrected_cents
    FROM seat_day_held
),
seat_day_step AS (
    SELECT
        *,
        corrected_cents - lagInFrame(corrected_cents, 1, toUInt32(0)) OVER (
            PARTITION BY tenant_id, source, source_id, account_id, period_month
            ORDER BY snapshot_date
        )                                       AS step_cents
    FROM seat_day_corrected
)

SELECT
    -- SAFETY: all three are safe under the WHERE in `seat_day_source`.
    assumeNotNull(tenant_id)                    AS tenant_id,
    source_id                                   AS source_id,
    account_id                                  AS account_id,
    assumeNotNull(email)                        AS email,
    assumeNotNull(snapshot_date)                AS snapshot_date,
    tool                                        AS tool,
    {{ ai_tool_label('tool') }}                 AS tool_label,
    coalesce(seat_tier, 'unknown')              AS seat_tier,
    toFloat64(step_cents) / 100                 AS daily_extra_usage_usd
FROM seat_day_step

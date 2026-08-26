-- depends_on: {{ ref('claude_team__bronze_promoted') }}

-- Per-seat reading of the month-to-date overage spend, one row per day the
-- endpoint was read. The sibling claude_team__ai_overage keeps the month's
-- closing state and the full seat state with it; this model keeps only what a
-- trajectory needs, so the two are not copies of one another.
--
-- The vendor reports a cumulative figure for the month in progress and no
-- history, so a day's spend is only ever the difference between two readings.
-- Differencing happens in gold, over the class this feeds: it needs the
-- neighbouring rows, and a correction that lowers the cumulative total has to
-- reach the days already emitted.
--
-- INVARIANT: full_refresh=false, for the same reason the monthly sibling
-- carries it. Bronze keys a reading by seat and day only since claude-team
-- 3.2.0; readings taken before that were overwritten within their month and
-- cannot be reproduced, so a rebuild from Bronze would silently shorten the
-- series rather than restore it.
--
-- INVARIANT: the Bronze read below carries no FINAL, and its dedup is scoped to
-- the DAY rather than to Bronze's own key. Both are deliberate. Before 3.2.0 the
-- key held the month, so every reading of a seat in a month is one row to the
-- table and ReplacingMergeTree keeps only the newest — but it collapses on merge,
-- not on insert, so readings that have not merged yet are still there to be read.
-- This model's first build is the one chance to keep them: FINAL would discard
-- exactly those rows, and a day-scoped LIMIT 1 BY keeps one per reading instead
-- of one per month. What survives is whatever ClickHouse has not merged, so this
-- rescues a tail and never a history.
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    unique_key='unique_key',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    on_schema_change='append_new_columns',
    settings={'allow_nullable_key': 1},
    full_refresh=false,
    schema='staging',
    tags=['claude-team', 'silver:class_ai_overage_daily']
) }}

WITH latest_per_seat_day AS (
    SELECT *
    FROM {{ source('bronze_claude_team', 'claude_team_overage_spend') }}
    WHERE account_uuid IS NOT NULL
      AND trim(account_uuid) != ''
      AND account_email IS NOT NULL
      AND trim(account_email) != ''
    ORDER BY _airbyte_extracted_at DESC
    -- Dedup on the full grain, as the monthly sibling does: one bronze schema
    -- can hold the same account_uuid under different tenant_id/source_id, and
    -- keying on the seat alone would drop those as false duplicates. The day
    -- comes from the extraction timestamp rather than from snapshot_date so
    -- rows written before 3.2.0 carried it in their key still land on a day.
    LIMIT 1 BY tenant_id, source_id, account_uuid, toDate(_airbyte_extracted_at)
)

SELECT
    tenant_id                                           AS insight_tenant_id,
    source_id,
    CAST(concat(
        coalesce(tenant_id, ''), '-',
        coalesce(source_id, ''), '-',
        coalesce(account_uuid, ''), '-',
        formatDateTime(toDate(_airbyte_extracted_at), '%Y-%m-%d')
    ) AS String)                                        AS unique_key,
    lower(trim(account_email))                          AS email,
    account_uuid                                        AS account_id,
    toDate(_airbyte_extracted_at)                       AS snapshot_date,
    toStartOfMonth(_airbyte_extracted_at)               AS period_month,
    'claude'                                            AS tool,
    seat_tier,
    coalesce(currency, 'USD')                           AS currency,
    -- Already cents (USD minor units) — NO ×100. round() guards against a
    -- float repr ('10000.0') that toUInt32OrNull would otherwise reject.
    toUInt32OrNull(toString(round(monthly_credit_limit))) AS credit_limit_cents,
    -- Cumulative for the billing month, as the vendor reports it. Not a day's
    -- spend: gold differences consecutive readings to get that.
    toUInt32(round(coalesce(used_credits, 0)))          AS used_amount_cents,
    'claude_team'                                       AS source,
    data_source,
    CAST(_airbyte_extracted_at AS Nullable(DateTime64(3))) AS collected_at,
    toUnixTimestamp64Milli(_airbyte_extracted_at)          AS _version
FROM latest_per_seat_day
{% if is_incremental() %}
  -- Re-read the current and previous month: a reading inside the current month
  -- still arrives daily, and a correction can land after a month has turned.
  WHERE toStartOfMonth(_airbyte_extracted_at) >= (
      SELECT coalesce(max(period_month), toDate('1970-01-01')) - INTERVAL 1 MONTH
      FROM {{ this }}
  )
{% endif %}

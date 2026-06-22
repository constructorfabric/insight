-- depends_on: {{ ref('claude_team__bronze_promoted') }}
-- Bronze → Silver: Claude Team per-seat credit spend vs limit → class_ai_overage
--
-- Source: bronze_claude_team.claude_team_overage_spend — the per-seat
-- spend-state snapshot pulled via the customer-deployed claude-team-proxy
-- from the claude.ai web API (/api/organizations/{org}/overage_spend_limits).
-- One row per seat (account_uuid). Requires the proxy sessionKey to hold
-- `billing:view` / Owner role — otherwise the Bronze stream is empty
-- (HTTP 403 IGNOREd upstream) and this model yields zero rows (sync GREEN).
--
-- This is the FIRST contributor to the class_ai_overage Silver class and
-- therefore DEFINES its 19-column positional contract (consumed by
-- `union_by_tag('silver:class_ai_overage')`). Any future source (OpenAI,
-- etc.) MUST emit these columns in this exact order — vendor-specific
-- fields go into overage_metrics_json, never new columns.
--
-- UNITS — CRITICAL: unlike claude_team__ai_dev_usage.cost_cents (which casts
-- a decimal-as-string dollar amount × 100), `used_credits` and
-- `monthly_credit_limit` here are ALREADY in minor units (cents, USD,
-- decimal_places=2): monthly_credit_limit=10000 ⇒ $100.00, used_credits=699
-- ⇒ $6.99. So credit_limit_cents / used_amount_cents map straight through
-- with NO ×100. Verified live (149 seats, currency='USD').
--
-- GRAIN: the endpoint reports current-billing-period-to-date spend with no
-- explicit period field. We stamp period_month = start-of-month of the
-- snapshot's extraction time and keep the LATEST snapshot per (seat, month).
-- As months roll over this accrues a monthly history; within the current
-- month it always reflects the freshest snapshot. unique_key carries the
-- month so a new month never overwrites a prior month's closing value.
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    unique_key='unique_key',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    on_schema_change='append_new_columns',
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['claude-team', 'silver:class_ai_overage']
) }}

WITH latest_per_seat_month AS (
    -- Bronze is full-refresh+append: every sync re-emits all seats with the
    -- same unique_key (no date). Collapse to the latest snapshot per seat per
    -- calendar month so each (seat, month) keeps its freshest spend reading.
    SELECT *
    FROM {{ source('bronze_claude_team', 'claude_team_overage_spend') }}
    WHERE account_uuid IS NOT NULL
      AND trim(account_uuid) != ''
      AND account_email IS NOT NULL
      AND trim(account_email) != ''
    ORDER BY _airbyte_extracted_at DESC
    -- Dedup on the FULL grain (tenant + source + seat + month), not just
    -- account_uuid: a multi-tenant / multi-instance bronze_claude_team can hold
    -- the same account_uuid under different tenant_id/source_id in one month,
    -- and keying on account_uuid alone would drop those as false duplicates.
    LIMIT 1 BY tenant_id, source_id, account_uuid, toStartOfMonth(_airbyte_extracted_at)
)

SELECT
    tenant_id                                           AS insight_tenant_id,
    source_id,
    -- Silver dedup key: tenant-source-seat-month. The month component (absent
    -- from the Bronze unique_key) preserves monthly history and gives intra-
    -- month idempotency (latest snapshot wins via _version, same key).
    CAST(concat(
        coalesce(tenant_id, ''), '-',
        coalesce(source_id, ''), '-',
        coalesce(account_uuid, ''), '-',
        formatDateTime(toStartOfMonth(_airbyte_extracted_at), '%Y-%m')
    ) AS String)                                        AS unique_key,
    -- Per-seat identity. Claude Team seats always carry an email; account_uuid
    -- is the stable vendor id (identity proxy / join anchor).
    lower(trim(account_email))                          AS email,
    account_uuid                                        AS account_id,
    toStartOfMonth(_airbyte_extracted_at)               AS period_month,
    'claude'                                            AS tool,
    seat_tier,
    coalesce(currency, 'USD')                           AS currency,
    -- Already cents (USD minor units) — NO ×100. NULL when no limit applies
    -- (e.g. unassigned seats with limit_type NULL). round() guards against a
    -- float repr ('10000.0') that toUInt32OrNull would otherwise reject.
    toUInt32OrNull(toString(round(monthly_credit_limit))) AS credit_limit_cents,
    toUInt32(round(coalesce(used_credits, 0)))          AS used_amount_cents,
    -- Overage = spend beyond the limit. NULL (not 0) when the limit is unknown
    -- — honest-NULL: we cannot compute overage without a limit.
    multiIf(
        monthly_credit_limit IS NULL, CAST(NULL AS Nullable(UInt32)),
        CAST(greatest(0, toInt64(round(coalesce(used_credits, 0)))
                         - toInt64(round(monthly_credit_limit))) AS Nullable(UInt32))
    )                                                   AS overage_cents,
    -- Soft over-limit flag (used > limit). NULL when limit unknown. Distinct
    -- from out_of_credits (hard exhaustion), which lives in the JSON blob.
    multiIf(
        monthly_credit_limit IS NULL, CAST(NULL AS Nullable(UInt8)),
        toUInt8(coalesce(used_credits, 0) > monthly_credit_limit)
    )                                                   AS is_over_limit,
    -- Bronze may store the JSON boolean as Bool, UInt8, or the strings
    -- 'true'/'false' depending on destination typing — normalise all forms.
    multiIf(
        lower(toString(is_enabled)) IN ('true', '1'),  toUInt8(1),
        lower(toString(is_enabled)) IN ('false', '0'), toUInt8(0),
        CAST(NULL AS Nullable(UInt8))
    )                                                   AS is_enabled,
    -- Vendor-specific extras kept out of the positional contract.
    toJSONString(map(
        'limit_type',         ifNull(toString(limit_type), ''),
        'used_credits_basis', ifNull(toString(used_credits_basis), ''),
        'out_of_credits',     ifNull(toString(out_of_credits), ''),
        'seat_tier',          ifNull(toString(seat_tier), '')
    ))                                                  AS overage_metrics_json,
    'claude_team'                                       AS source,
    data_source,
    CAST(_airbyte_extracted_at AS Nullable(DateTime64(3))) AS collected_at,
    toUnixTimestamp64Milli(_airbyte_extracted_at)          AS _version
FROM latest_per_seat_month
{% if is_incremental() %}
  -- Re-evaluate the current and previous month so an in-flight month's
  -- closing value keeps updating; older months are immutable.
  WHERE toStartOfMonth(_airbyte_extracted_at) >= (
      SELECT coalesce(max(period_month), toDate('1970-01-01')) - INTERVAL 1 MONTH
      FROM {{ this }}
  )
{% endif %}

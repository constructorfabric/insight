-- depends_on: {{ ref('github_copilot__bronze_promoted') }}
-- Bronze → Silver: GitHub Copilot org-level daily aggregates.
--
-- DEFERRED — `silver.class_ai_org_usage` does NOT yet exist (no schema
-- entry in `silver/ai/schema.yml`, no `silver/ai/class_ai_org_usage.sql`).
-- Copilot is the proposed first contributor (see DESIGN §3.7 and
-- `copilot_org_metrics` table reference). Until the class is created in
-- a coordinated PR, this staging ships with `enabled=false` and is not
-- materialised.
--
-- Activation steps (tracked under PRD OQ-COP-2):
--   1. File a GitHub issue under constructorfabric/insight to track the new
--      Silver class creation.
--   2. Create `silver/ai/class_ai_org_usage.sql` with
--      `engine='ReplacingMergeTree(_version)'` and
--      `order_by=['unique_key']` per ADR-0001.
--   3. Add the class definition to `silver/ai/schema.yml` with not_null
--      tests on tenant/source/unique_key/day and accepted_values for
--      `source` and `data_source` enum.
--   4. Drop `enabled=false` from this model's config.
--   5. Run `dbt run --select tag:github-copilot+` to materialise the
--      first batch.
--
-- Per ADR-0001, when activated this model emits a `_version` column
-- (`toUnixTimestamp64Milli(_airbyte_extracted_at)`) so the silver RMT
-- deduplicates by `unique_key`.

{{ config(
    enabled=false,
    materialized='incremental',
    incremental_strategy='append',
    unique_key='unique_key',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    on_schema_change='append_new_columns',
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['github-copilot', 'silver:class_ai_org_usage']
) }}

WITH org_metrics AS (
    SELECT *
    FROM {{ source('bronze_github_copilot', 'copilot_org_metrics') }}
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY tenant_id, source_id, day
)

SELECT
    tenant_id                                                              AS insight_tenant_id,
    source_id,
    CAST(concat(
        coalesce(tenant_id, ''), '-',
        coalesce(source_id, ''), '-',
        coalesce(day, '')
    ) AS String)                                                           AS unique_key,
    toDate(parseDateTimeBestEffortOrNull(day))                             AS day,
    organization_id,
    enterprise_id,
    -- Active-user aggregates surfaced by the v2 org metrics endpoint.
    -- The API exposes daily/weekly/monthly active counters across
    -- multiple Copilot surfaces — we forward all of them as nullable
    -- so the future Silver class can pick whichever cadence it needs.
    toUInt32(coalesce(daily_active_users, 0))                              AS daily_active_users,
    toUInt32(coalesce(weekly_active_users, 0))                             AS weekly_active_users,
    toUInt32(coalesce(monthly_active_users, 0))                            AS monthly_active_users,
    CAST(monthly_active_chat_users AS Nullable(UInt32))                    AS monthly_active_chat_users,
    CAST(monthly_active_agent_users AS Nullable(UInt32))                   AS monthly_active_agent_users,
    -- Interaction / code metrics (org-wide totals).
    toUInt32(coalesce(user_initiated_interaction_count, 0))                AS user_initiated_interaction_count,
    toUInt32(coalesce(code_generation_activity_count, 0))                  AS code_generation_activity_count,
    toUInt32(coalesce(code_acceptance_activity_count, 0))                  AS code_acceptance_activity_count,
    toUInt32(coalesce(loc_added_sum, 0))                                   AS loc_added_sum,
    toUInt32(coalesce(loc_deleted_sum, 0))                                 AS loc_deleted_sum,
    -- Pull-request object passed through as JSON for downstream parsing.
    pull_requests                                                          AS pull_requests_json,
    'copilot'                                                              AS source,
    'insight_github_copilot'                                               AS data_source,
    parseDateTime64BestEffortOrNull(coalesce(collected_at, ''), 3)         AS collected_at,
    toUnixTimestamp64Milli(_airbyte_extracted_at)                          AS _version
FROM org_metrics
WHERE day IS NOT NULL AND day != ''
{% if is_incremental() %}
  -- 7-day re-process window: must be >= the connector's metrics_lookback_days
  -- (default 7, source_github_copilot) so days the connector re-fetches when a
  -- Copilot report lands late or is restated actually reach Silver. A narrower
  -- window would let those days sit in Bronze but be dropped here (older than
  -- max(day) - N), wasting the connector lookback. (Refs #1354.)
  AND toDate(parseDateTimeBestEffortOrNull(day)) > (
      SELECT coalesce(max(day), toDate('1970-01-01')) - INTERVAL 7 DAY
      FROM {{ this }}
  )
{% endif %}

-- depends_on: {{ ref('chatgpt_team__bronze_promoted') }}
-- Bronze → Silver: ChatGPT Team per-user per-day Codex credits → class_ai_credit_usage.
--
-- Source: bronze_chatgpt_team.chatgpt_team_codex_user_daily, one row per
-- (email, date).
--
-- INVARIANT: this model does NOT share chatgpt_team__ai_dev_usage's emission
-- filter, and must not acquire one. That model admits a person-day on
-- n_threads or lines_added because its contract derives active_day from a row
-- existing; credits evidence spend, not activity, and the two person-days
-- disagree often enough to matter — measured against a workspace, credit-
-- bearing days that the activity filter refuses carry a low-single-digit
-- percentage of all credits. Routing money through that model would lose
-- exactly those. Every row the vendor reports with credits above zero belongs
-- here, whatever else the person did that day.
--
-- INVARIANT: `credits` is ON-DEMAND usage, not total Codex consumption. It
-- equals the vendor's own on-demand figure at every grain that publishes one,
-- and Codex activity routinely records zero credits while still reporting
-- tokens. Whether the uncredited part is allowance-covered or unmetered is not
-- established; either way it is excluded, so this is a floor on consumption and
-- the exact thing an on-demand charge is levied on.
--
-- No money here. Credits are a vendor-internal unit; gold multiplies them by
-- config.ai_credit_pricing at read time and never stores the product.
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    unique_key='unique_key',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    on_schema_change='append_new_columns',
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['chatgpt-team', 'silver:class_ai_credit_usage']
) }}

{% if is_incremental() %}
-- One watermark per connector instance. A global max(day) lets a busy instance
-- raise the floor for a quiet one and silently skip its days; the floor has to
-- belong to the instance it filters. Grouped once and joined, rather than a
-- correlated scalar subquery, which ClickHouse supports unevenly.
WITH instance_watermark AS (

    SELECT
        insight_tenant_id,
        source_id,
        max(day)                                        AS latest_day
    FROM {{ this }}
    GROUP BY insight_tenant_id, source_id

)
{% endif %}
SELECT
    bronze.tenant_id                                    AS insight_tenant_id,
    bronze.source_id                                    AS source_id,
    CAST(concat(
        coalesce(tenant_id, ''), '-',
        coalesce(source_id, ''), '-',
        lower(trim(coalesce(email, ''))), '-',
        coalesce(date, '')
    ) AS String)                                        AS unique_key,
    lower(trim(email))                                  AS email,
    -- assumeNotNull after the NOT NULL filter below: gold builds record_id
    -- from toString(day), and a Nullable there reaches the evidence table's
    -- sorting key, which MergeTree refuses.
    assumeNotNull(toDate(date))                         AS day,
    'codex'                                             AS tool,
    -- Decimal, not Float: credits are summed across people and months, and a
    -- float sum reorders under parallel aggregation and stops being stable.
    CAST(coalesce(toDecimal64OrNull(toString(credits), 6), 0) AS Decimal(18, 6)) AS credits,
    -- The vendor's own designation for these credits, not a claim about what
    -- they sit on top of: whether the uncredited part is allowance-covered or
    -- unmetered is not established.
    'on_demand'                                         AS credit_kind,
    'chatgpt_team'                                      AS source,
    bronze.data_source,
    CAST(_airbyte_extracted_at AS Nullable(DateTime64(3))) AS collected_at,
    toUnixTimestamp64Milli(_airbyte_extracted_at)          AS _version
FROM (
    -- Bronze dedup on the SAME normalized key the unique_key uses, else two
    -- case-variant spellings of one address both survive and collide.
    SELECT *
    FROM {{ source('bronze_chatgpt_team', 'chatgpt_team_codex_user_daily') }}
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY tenant_id, source_id, lower(trim(email)), date
) AS bronze
{% if is_incremental() %}
LEFT JOIN instance_watermark AS w
       ON w.insight_tenant_id = bronze.tenant_id
      AND w.source_id = bronze.source_id
{% endif %}
WHERE email IS NOT NULL
  AND trim(email) != ''
  AND date IS NOT NULL
  AND trim(date) != ''
  -- INVARIANT: a zero-credit person-day IS emitted. The vendor can revise a
  -- charge down to nothing, and this relation is a ReplacingMergeTree keyed on
  -- unique_key — so the corrected zero has to arrive as a row, or the earlier
  -- positive one stays the stored truth forever. Suppressing it here would
  -- leave a charge nobody can withdraw.
  --
  -- The correction reaches Bronze only while the connector still re-reads the
  -- day (lookback_window P2D); the 3-day window below is wider than that, so
  -- it is the vendor's re-read, not this filter, that bounds how late a
  -- revision can land.
  --
  -- Gold admits the money, not this model: the monetary branch of
  -- ai_cost_metric_evidence requires credits > 0, so a zero never becomes a
  -- $0 charge.
{% if is_incremental() %}
  -- An instance this relation has never held reads everything: the LEFT JOIN
  -- leaves latest_day NULL for it, and the whole predicate is skipped. One it
  -- has read before is filtered by ITS OWN floor, three days back, which is the
  -- same window as before — only no longer shared with every other instance.
  AND (
    w.latest_day IS NULL
    OR toDate(bronze.date) > w.latest_day - INTERVAL 3 DAY
  )
{% endif %}

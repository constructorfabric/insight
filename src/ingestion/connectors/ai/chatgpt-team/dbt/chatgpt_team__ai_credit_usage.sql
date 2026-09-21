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
-- config.ai_credit_price at read time and never stores the product.
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

SELECT
    tenant_id                                           AS insight_tenant_id,
    source_id,
    CAST(concat(
        coalesce(tenant_id, ''), '-',
        coalesce(source_id, ''), '-',
        lower(trim(coalesce(email, ''))), '-',
        coalesce(date, '')
    ) AS String)                                        AS unique_key,
    lower(trim(email))                                  AS email,
    toDate(date)                                        AS day,
    'codex'                                             AS tool,
    -- Decimal, not Float: credits are summed across people and months, and a
    -- float sum reorders under parallel aggregation and stops being stable.
    CAST(coalesce(toDecimal64OrNull(toString(credits), 6), 0) AS Decimal(18, 6)) AS credits,
    'on_demand'                                         AS credit_kind,
    'chatgpt_team'                                      AS source,
    data_source,
    CAST(_airbyte_extracted_at AS Nullable(DateTime64(3))) AS collected_at,
    toUnixTimestamp64Milli(_airbyte_extracted_at)          AS _version
FROM (
    -- Bronze dedup on the SAME normalized key the unique_key uses, else two
    -- case-variant spellings of one address both survive and collide.
    SELECT *
    FROM {{ source('bronze_chatgpt_team', 'chatgpt_team_codex_user_daily') }}
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY tenant_id, source_id, lower(trim(email)), date
)
WHERE email IS NOT NULL
  AND trim(email) != ''
  AND date IS NOT NULL
  AND trim(date) != ''
  -- A zero-credit person-day is a fact the vendor states, but it is not a
  -- charge and nothing downstream distinguishes it from an absent row.
  AND coalesce(toDecimal64OrNull(toString(credits), 6), 0) > 0
{% if is_incremental() %}
  -- Empty-table guard, as in chatgpt_team__ai_dev_usage: over an empty `this`
  -- max(day) is the Date epoch and the interval underflows, filtering
  -- everything out.
  AND (
    (SELECT count() FROM {{ this }}) = 0
    OR toDate(date) > (
        SELECT coalesce(max(day), toDate('1970-01-01')) - INTERVAL 3 DAY
        FROM {{ this }}
    )
  )
{% endif %}

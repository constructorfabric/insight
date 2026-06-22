{{ config(
    materialized='incremental',
    incremental_strategy='delete+insert',
    unique_key='unique_key',
    schema='silver',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['silver']
) }}

-- Unified per-person per-month AI spend-vs-limit (overage) across vendors.
-- Grain: one row per (tenant, source, seat, billing month). Homogeneous
-- monetary contract in minor units (cents) + ISO currency so Claude Team and
-- (future) OpenAI / ChatGPT seats are directly comparable. overage_cents =
-- max(0, used_amount_cents - credit_limit_cents), NULL when no limit is known.
--
-- depends_on: {{ ref('claude_team__ai_overage') }}

SELECT * FROM (
    {{ union_by_tag('silver:class_ai_overage') }}
)
{% if is_incremental() %}
WHERE _version > (SELECT max(_version) FROM {{ this }})
{% endif %}

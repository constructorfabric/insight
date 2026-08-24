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

-- Per-person readings of the month-to-date AI overage spend, across vendors.
-- Grain: one row per (tenant, source, seat, day the vendor was read). Same
-- monetary contract as class_ai_overage — minor units (cents) + ISO currency.
--
-- The sibling class_ai_overage answers "what did this seat spend this month";
-- this one answers "how did it get there". A vendor that reports a cumulative
-- month-to-date figure gives a day's spend only as the difference between two
-- readings, so the readings have to survive as rows. Differencing is gold's
-- job: it needs neighbouring rows, and a correction lowering the cumulative
-- total has to reach days already emitted.
--
-- depends_on: {{ ref('claude_team__ai_overage_daily') }}

SELECT candidate.* FROM (
    {{ union_by_tag('silver:class_ai_overage_daily') }}
) AS candidate
{{ silver_incremental_watermark(['insight_tenant_id', 'source_id']) }}

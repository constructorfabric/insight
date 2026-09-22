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

-- Per-person readings of AI usage credits, across vendors. Grain: one row per
-- (tenant, source, person, day).
--
-- A credit is a vendor-internal consumption unit, not money. It is kept as a
-- count here and priced only at read time, from config.ai_credit_pricing — so a
-- restated price restates the whole history instead of leaving stored amounts
-- that disagree with the rate that produced them.
--
-- Distinct from class_ai_overage and class_ai_overage_daily, which carry money
-- the vendor itself denominated (minor units plus an ISO currency) as a
-- cumulative month-to-date level. This relation carries a per-day count that is
-- summable, and nothing in it is a currency.
--
-- INVARIANT: contributors admit a row on credits alone and never on an activity
-- counter. A person-day can carry credits while every activity counter its
-- vendor publishes reads zero — continuing an existing session spends without
-- starting one — and a contributor that filtered on activity would drop real
-- charges. In particular this relation is never sourced from
-- class_ai_dev_usage, whose emission filter does exactly that.
--
-- depends_on: {{ ref('chatgpt_team__ai_credit_usage') }}

SELECT candidate.* FROM (
    {{ union_by_tag('silver:class_ai_credit_usage') }}
) AS candidate
{{ silver_incremental_watermark(['insight_tenant_id', 'source_id']) }}

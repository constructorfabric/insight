-- depends_on: {{ ref('zendesk__support_agent') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    incremental_strategy='delete+insert',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='silver',
    tags=['silver']
) }}

-- Cross-vendor support-agent dimension (agent → Insight person via
-- lower(email)). Union of per-source staging models tagged
-- `silver:dim_support_agent`. Joined to fct/activity by (data_source,
-- source_agent_id) to resolve the actor of an event to a person.
SELECT candidate.* FROM (
    {{ union_by_tag('silver:dim_support_agent') }}
) AS candidate
{{ silver_incremental_watermark(['tenant_id', 'insight_source_id', 'data_source']) }}

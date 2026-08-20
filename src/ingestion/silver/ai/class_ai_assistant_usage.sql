{{ config(
    materialized='incremental',
    incremental_strategy='delete+insert',
    unique_key='unique_key',
    schema='silver',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    on_schema_change='append_new_columns',
    settings={'allow_nullable_key': 1},
    tags=['silver']
) }}

-- depends_on: {{ ref('claude_enterprise__ai_assistant_usage') }}
-- depends_on: {{ ref('chatgpt_team__ai_assistant_usage') }}

SELECT candidate.* FROM (
    {{ union_by_tag('silver:class_ai_assistant_usage') }}
) AS candidate
{{ silver_incremental_watermark(['insight_tenant_id', 'source_id']) }}

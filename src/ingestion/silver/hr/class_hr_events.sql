-- depends_on: {{ ref('bamboohr__hr_events') }}

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

SELECT candidate.* FROM (
    {{ union_by_tag('silver:class_hr_events') }}
) AS candidate
{{ silver_incremental_watermark(['insight_tenant_id', 'source_id', 'source']) }}

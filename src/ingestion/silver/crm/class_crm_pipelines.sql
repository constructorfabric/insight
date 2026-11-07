-- depends_on: {{ ref('hubspot__crm_pipelines') }}
{{ config(
    materialized='incremental',
    incremental_strategy='delete+insert',
    unique_key='unique_key',
    schema='silver',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['silver']
) }}

SELECT candidate.* FROM (
    {{ union_by_tag('silver:class_crm_pipelines') }}
) AS candidate
{{ silver_incremental_watermark(['tenant_id', 'source_id']) }}

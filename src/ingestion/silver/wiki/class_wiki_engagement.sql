{{ config(
    materialized='incremental',
    incremental_strategy='delete+insert',
    unique_key='unique_key',
    schema='silver',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['silver']
) }}

-- depends_on: {{ ref('confluence__wiki_engagement') }}
-- depends_on: {{ ref('outline__wiki_engagement') }}

SELECT candidate.* FROM (
    {{ union_by_tag('silver:class_wiki_engagement') }}
) AS candidate
{{ silver_incremental_watermark(['tenant_id', 'source_id', 'data_source']) }}

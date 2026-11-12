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

-- depends_on: {{ ref('hubspot__crm_deals') }}

WITH src AS (
    {{ union_by_tag('silver:class_crm_deals') }}
)
SELECT src.*
FROM src
{{ silver_incremental_watermark(['unique_key'], alias='src') }}

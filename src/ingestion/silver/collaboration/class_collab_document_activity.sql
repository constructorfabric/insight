-- depends_on: {{ ref('m365__collab_document_activity_onedrive') }}
-- depends_on: {{ ref('m365__collab_document_activity_sharepoint') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    incremental_strategy='delete+insert',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='silver',
    tags=['silver']
) }}

SELECT candidate.* FROM (
    {{ union_by_tag('silver:class_collab_document_activity') }}
) AS candidate
{{ silver_incremental_watermark(['tenant_id', 'insight_source_id', 'data_source', 'product']) }}

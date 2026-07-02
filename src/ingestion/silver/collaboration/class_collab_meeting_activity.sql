-- depends_on: {{ ref('m365__collab_meeting_activity') }}
-- depends_on: {{ ref('zoom__collab_meeting_activity') }}
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

{{ incremental_watermark(union_by_tag('silver:class_collab_meeting_activity'), tenant_col='tenant_id', source_col='insight_source_id') }}

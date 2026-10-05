-- depends_on: {{ ref('jira__task_users') }}
-- depends_on: {{ ref('github__task_users') }}
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
    {{ union_by_tag('silver:class_task_users') }}
) AS candidate
{{ silver_incremental_watermark(['tenant_id', 'insight_source_id', 'data_source']) }}

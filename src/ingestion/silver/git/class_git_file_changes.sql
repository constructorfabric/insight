-- depends_on: {{ ref('github__file_changes') }}
-- depends_on: {{ ref('bitbucket_cloud__file_changes') }}
-- depends_on: {{ ref('gitlab__file_changes') }}
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

SELECT candidate.* FROM (
    {{ union_by_tag('silver:class_git_file_changes') }}
) AS candidate
{{ silver_incremental_watermark(['tenant_id', 'source_id', 'data_source']) }}

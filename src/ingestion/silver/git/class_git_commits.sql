-- depends_on: {{ ref('github__commits') }}
-- depends_on: {{ ref('bitbucket_cloud__commits') }}
-- depends_on: {{ ref('gitlab__commits') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    incremental_strategy='delete+insert',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by=['unique_key'],
    on_schema_change='append_new_columns',
    settings={'allow_nullable_key': 1},
    schema='silver',
    tags=['silver']
) }}

SELECT candidate.* FROM (
    {{ union_by_tag('silver:class_git_commits') }}
) AS candidate
{{ silver_incremental_watermark(['tenant_id', 'source_id', 'data_source']) }}

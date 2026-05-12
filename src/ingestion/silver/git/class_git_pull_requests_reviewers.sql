-- depends_on: {{ ref('github__pull_requests_reviewers') }}
-- depends_on: {{ ref('bitbucket_cloud__pull_requests_reviewers') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    incremental_strategy='append',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='silver',
    tags=['silver']
) }}

SELECT * FROM (
    {{ union_by_tag('silver:class_git_pull_requests_reviewers') }}
)
{% if is_incremental() %}
WHERE _version > (SELECT max(_version) FROM {{ this }})
{% endif %}

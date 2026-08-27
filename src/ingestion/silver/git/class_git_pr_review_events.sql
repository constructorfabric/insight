-- depends_on: {{ ref('github__pr_review_events') }}
-- depends_on: {{ ref('bitbucket_cloud__pr_review_events') }}
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

SELECT * FROM (
    {{ union_by_tag('silver:class_git_pr_review_events') }}
)
{% if is_incremental() %}
WHERE _version > (SELECT max(_version) FROM {{ this }})
{% endif %}

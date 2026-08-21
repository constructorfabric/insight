-- depends_on: {{ ref('jira__task_issuetypes') }}
-- depends_on: {{ ref('github__task_issuetypes') }}
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

-- Unified, source-neutral issue-type dimension: one row per source issue type,
-- carrying the reconciled `issue_kind` (bug / other / unknown). Each per-source
-- projection tagged `silver:class_task_issuetypes` reconciles its native type
-- naming to the same enum, so Gold reads one column and matches no type name.

SELECT * FROM (
    {{ union_by_tag('silver:class_task_issuetypes') }}
)
{% if is_incremental() %}
WHERE _version > (SELECT max(_version) FROM {{ this }})
{% endif %}

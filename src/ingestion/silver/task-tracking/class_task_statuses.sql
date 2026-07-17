-- depends_on: {{ ref('jira__task_statuses') }}
-- depends_on: {{ ref('youtrack__task_statuses') }}
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

-- Unified, source-neutral status dimension: one row per (source status id),
-- carrying the reconciled lifecycle `status_category` (new / in_progress /
-- done / undefined). Each per-source projection tagged `silver:class_task_statuses`
-- (jira__task_statuses, youtrack__task_statuses) reconciles its native signal —
-- Jira `statusCategory`, YouTrack `isResolved` — to the SAME enum, so after the
-- union there is no cross-source divergence. Gold detects a closed task with
-- `status_category = 'done'`, never a localized status name. See issue #1541.

SELECT * FROM (
    {{ union_by_tag('silver:class_task_statuses') }}
)
{% if is_incremental() %}
WHERE _version > (SELECT max(_version) FROM {{ this }})
{% endif %}

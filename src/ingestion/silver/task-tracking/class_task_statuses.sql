-- depends_on: {{ ref('youtrack__task_statuses') }}
-- depends_on: {{ ref('jira__task_statuses') }}
-- depends_on: {{ ref('github__task_statuses') }}
{{ config(
    materialized='incremental',
    pre_hook="{{ youtrack_reconcile_class() }}",
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

SELECT candidate.* FROM (
    {{ union_by_tag('silver:class_task_statuses') }}
) AS candidate
-- YouTrack is reconciled whole by the pre_hook, so its rows bypass the boundary.
{{ silver_incremental_watermark(['insight_source_id', 'data_source'], always_reread="candidate.data_source = 'youtrack'") }}

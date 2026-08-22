-- depends_on: {{ ref('github__item_events') }}
-- depends_on: {{ ref('bitbucket_cloud__item_events') }}
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

-- Per-(item × field × event) lifecycle history for pull requests:
-- when an item changed state, who changed it, and to what. One row per vendor
-- event, keyed on the vendor's own event id.
--
-- Deltas only: `delta_action` plus the value it applies, never a post-event
-- value set. Reconstructing the state of a multi-valued field (label,
-- assignee, reviewer) is a fold the consumer performs; for the single-valued
-- fields the delta IS the state. `class_task_field_history` carries the folded
-- arrays instead because a dedicated enrich binary computes them.
--
-- `prev_value_id` is NULL throughout: no pull-request event reports where its
-- change came from. An empty string would claim the previous value was empty.
--
-- Issue events are NOT here. An issue's lifecycle is a task tracker's history
-- and reaches `class_task_field_history`; carrying it in both places would be
-- two records of the same facts, only one of which feeds a metric.

SELECT * FROM (
    {{ union_by_tag('silver:class_git_item_events') }}
)
{% if is_incremental() %}
WHERE _version > (SELECT max(_version) FROM {{ this }})
{% endif %}

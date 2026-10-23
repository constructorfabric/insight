-- depends_on: {{ ref('jira__task_field_text') }}
-- depends_on: {{ ref('youtrack__task_field_text') }}
{{ config(
    materialized='incremental',
    pre_hook=[
        "{{ reconcile_class_from_producer('jira') }}",
        "{{ youtrack_reconcile_class() }}",
    ],
    incremental_strategy='delete+insert',
    unique_key='unique_key',
    schema='silver',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by=['unique_key'],
    tags=['silver']
) }}

-- Bodies of long-text field values, which `class_task_field_history` refers to
-- by `text_id` (the `task_field_text` macros). Producers union in via the
-- `silver:class_task_field_text` tag, and the pre_hooks drop a body its producer
-- no longer holds.

SELECT candidate.* FROM (
    {{ union_by_tag('silver:class_task_field_text') }}
) AS candidate
{% if is_incremental() %}
-- INVARIANT: a row is a pure function of its key, so a key already stored never
-- needs rewriting; every producer is rebuilt whole and re-stamps `_version`.
WHERE candidate.unique_key NOT IN (SELECT unique_key FROM {{ this }})
{% endif %}

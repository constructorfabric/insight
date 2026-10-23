-- depends_on: {{ ref('jira__task_field_current_text') }}
-- depends_on: {{ ref('youtrack__task_field_current_text') }}
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

-- The current body of each long-text field of an issue, by the issue's immutable
-- id: a `text_id` into `class_task_field_text`. Every producer is rebuilt whole,
-- so the union is read whole and the pre_hooks drop a field that was emptied.

SELECT candidate.* FROM (
    {{ union_by_tag('silver:class_task_field_current_text') }}
) AS candidate

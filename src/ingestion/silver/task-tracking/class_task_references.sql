-- depends_on: {{ ref('jira__task_references') }}
{{ config(
    materialized='incremental',
    pre_hook=[
        "{{ reconcile_class_from_producer('jira') }}",
    ],
    incremental_strategy='delete+insert',
    unique_key='unique_key',
    schema='silver',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['silver']
) }}

-- Which issues an issue points at, and where: a link, a key written in a field,
-- a key written in a comment. Every producer is rebuilt whole, so the union is
-- read whole and the pre_hook drops a reference that went away.

SELECT candidate.* FROM (
    {{ union_by_tag('silver:class_task_references') }}
) AS candidate

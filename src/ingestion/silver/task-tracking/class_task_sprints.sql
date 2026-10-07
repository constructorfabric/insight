-- depends_on: {{ ref('youtrack__task_sprints') }}
-- depends_on: {{ ref('jira__task_sprints') }}
{{ config(
    materialized='incremental',
    pre_hook="{{ youtrack_reconcile_class() }}",
    incremental_strategy='delete+insert',
    unique_key='unique_key',
    schema='silver',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['silver']
) }}

SELECT candidate.* FROM (
    {{ union_by_tag('silver:class_task_sprints') }}
) AS candidate
-- YouTrack is reconciled whole by the pre_hook, so its rows bypass the boundary.
{{ silver_incremental_watermark(['insight_source_id', 'data_source'], always_reread="candidate.data_source = 'youtrack'") }}

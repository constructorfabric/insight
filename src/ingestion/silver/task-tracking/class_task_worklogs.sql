-- depends_on: {{ ref('youtrack__task_worklogs') }}
-- depends_on: {{ ref('jira__task_worklogs') }}
{{ config(
    materialized='incremental',
    on_schema_change='append_new_columns',
    pre_hook="{{ youtrack_reconcile_class() }}",
    incremental_strategy='delete+insert',
    unique_key='unique_key',
    schema='silver',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['silver']
) }}

SELECT candidate.* FROM (
    {{ union_by_tag('silver:class_task_worklogs') }}
) AS candidate
-- YouTrack is reconciled whole by the pre_hook, so its rows bypass the boundary.
{{ silver_incremental_watermark(['insight_source_id', 'data_source'], always_reread="candidate.data_source = 'youtrack'") }}

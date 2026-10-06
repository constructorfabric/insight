-- depends_on: {{ ref('youtrack__task_issuetypes') }}
-- depends_on: {{ ref('jira__task_issuetypes') }}
-- depends_on: {{ ref('github__task_issuetypes') }}
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

-- Unified, source-neutral issue-type dimension: one row per source issue type,
-- raw vendor identity only (id and names). Each per-source projection tagged
-- `silver:class_task_issuetypes` carries the same columns; classification into
-- an issue kind is NOT here — gold resolves it from `config.field_value_map`
-- at its own build, so a mapping change never requires a silver rebuild.

SELECT candidate.* FROM (
    {{ union_by_tag('silver:class_task_issuetypes') }}
) AS candidate
-- YouTrack is reconciled whole by the pre_hook, so its rows bypass the boundary.
{{ silver_incremental_watermark(['insight_source_id', 'data_source'], always_reread="candidate.data_source = 'youtrack'") }}

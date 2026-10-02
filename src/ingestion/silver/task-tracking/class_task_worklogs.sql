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

SELECT * FROM (
    {{ union_by_tag('silver:class_task_worklogs') }}
)
{% if is_incremental() %}
-- YouTrack is reconciled whole by the pre_hook. The watermark is every other
-- vendor's own: YouTrack rows are versioned at build time, so a max over the
-- whole class would run ahead of a vendor's pending, older evidence.
WHERE data_source = 'youtrack'
   OR _version > (SELECT max(_version) FROM {{ this }} WHERE data_source != 'youtrack')
{% endif %}

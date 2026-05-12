-- depends_on: {{ ref('jira__task_field_history') }}
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='silver',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['silver']
) }}

-- Event-sourced per-(issue × field × event) history. Per ADR-005, synthetic_initial
-- rows share `event_id` (`initial:<issue_id>`) across fields of one issue, and
-- real-change rows share `event_id = changelog_id` across fields of one changelog
-- — both are disambiguated by `field_id`. The dedup grain is therefore
-- (insight_source_id, data_source, id_readable, field_id, event_id).
-- Per the project-wide convention this composite is encoded into `unique_key` by
-- the staging view (`jira__task_field_history.sql`), and that single column is the
-- ORDER BY here.

-- Source of truth is the Rust `jira-enrich` binary, which writes
-- `staging.jira__task_field_history` — see `jira__task_field_history.sql` for
-- the thin view that exposes it here.

SELECT * FROM (
    {{ union_by_tag('silver:class_task_field_history') }}
)
{% if is_incremental() %}
WHERE _version > (SELECT max(_version) FROM {{ this }})
{% endif %}

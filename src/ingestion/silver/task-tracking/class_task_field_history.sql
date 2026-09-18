-- depends_on: {{ ref('jira__field_history_derived') }}
-- depends_on: {{ ref('jira__availability_events') }}
-- depends_on: {{ ref('jira__comment_lifecycle_events') }}
-- depends_on: {{ ref('jira__worklog_lifecycle_events') }}
-- depends_on: {{ ref('github__task_field_history') }}
{{ config(
    materialized='incremental',
    incremental_strategy='delete+insert',
    unique_key='unique_key',
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
-- (insight_source_id, data_source, issue_id, field_id, event_id). The issue is
-- named by `issue_id`, which the tracker never reissues; `id_readable` changes
-- when a repository is renamed or an issue moves between projects, and a key
-- built from it stored that issue's whole history twice with nothing for
-- ReplacingMergeTree to collapse (#2741). It rides along as an attribute and is
-- resolved per issue by the latest event.
-- Per the project-wide convention this composite is encoded into `unique_key` by
-- each producer (`jira_history_key` for the Jira arms), and that single column is
-- the ORDER BY here.

-- Producers union in via the `silver:class_task_field_history` tag: the Jira
-- journal derived in dbt (`jira__field_history_derived`), the jira availability
-- and comment/worklog lifecycle event models, and the GitHub Issues arm. Every
-- producer is a dbt model; the decision once cited as ADR-003, that a Rust
-- binary owns the Jira journal, is reversed — see FIELD-HISTORY-IN-DBT.md §10.
--
-- The discriminator columns (`event_kind`, `field_cardinality`, `delta_action`,
-- `value_id_type`) are `LowCardinality(String)`: an enum would couple every
-- source's arm to the values of all the others. The accepted values are data
-- tests in schema.yml.

SELECT * FROM (
    {{ union_by_tag('silver:class_task_field_history') }}
)
{% if is_incremental() %}
WHERE _version > (SELECT max(_version) FROM {{ this }})
{% endif %}

-- depends_on: {{ ref('jira__bronze_promoted') }}
-- @cpt-principle:cpt-dataflow-principle-ephemeral-passthrough:p1
{{ config(
    materialized='ephemeral',
    tags=['jira', 'silver:class_task_field_history']
) }}

-- Ephemeral: this model creates NO database object. It exists only to attach
-- the `silver:class_task_field_history` tag so `union_by_tag` finds it in the
-- silver model. dbt inlines the SELECT as a CTE wherever it's `ref`'d.
--
-- The underlying staging table `staging.jira__task_field_history` is written
-- by the Rust `jira-enrich` binary; its DDL is managed by the
-- `create_task_field_history_staging` macro (see `on-run-start` in
-- `dbt_project.yml`). Rust populates `unique_key` per the convention
-- `{insight_source_id}-{data_source}-{id_readable}-{field_id}-{event_id}`
-- — see src/ingestion/connectors/task-tracking/jira/enrich/src/io/writer.rs.
--
-- event_kind is recast to the class contract's superset enum: the Rust table
-- keeps its two values, but the union sibling jira__availability_events also
-- emits 'availability' rows and UNION ALL needs one common enum type.

SELECT
    unique_key,
    insight_source_id,
    data_source,
    issue_id,
    id_readable,
    title,
    event_id,
    event_at,
    CAST(event_kind, 'Enum8(\'changelog\' = 1, \'synthetic_initial\' = 2, \'availability\' = 3, \'lifecycle\' = 4)')
                        AS event_kind,
    _seq,
    author_id,
    author_display,
    field_id,
    field_name,
    field_cardinality,
    delta_action,
    delta_value_id,
    delta_value_display,
    value_ids,
    value_displays,
    value_id_type,
    collected_at,
    _version
FROM {{ source('staging_jira', 'jira__task_field_history') }}

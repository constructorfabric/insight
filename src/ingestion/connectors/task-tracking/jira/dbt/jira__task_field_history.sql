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

SELECT * FROM {{ source('staging_jira', 'jira__task_field_history') }}

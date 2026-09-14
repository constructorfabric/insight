-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='view',
    alias='jira__task_field_kind',
    schema='staging',
    tags=['jira', 'staging']
) }}

-- Per-source field classification: one row per (source, field) carrying the
-- `field_kind` that decides how the field's value is normalized and how its
-- changelog deltas are applied. See
-- `connectors/task-tracking/jira/specs/FIELD-HISTORY-IN-DBT.md` §3.
--
-- View: the classification is a pure function of the current field catalogue,
-- so the current state of bronze is the current state of staging. Bronze
-- `jira_fields` is RMT-promoted (`jira__bronze_promoted`) keyed by
-- `unique_key`, hence FINAL on the read.
--
-- 'staging' + 'jira' tags are required: the pipeline's staging phase selects
-- the `tag:staging,tag:jira` intersection, and the models that consume this one
-- run in that phase.

SELECT
    COALESCE(f.source_id, '')             AS insight_source_id,
    CAST('jira' AS String)                AS data_source,
    COALESCE(f.field_id, '')              AS field_id,
    COALESCE(f.name, '')                  AS field_name,
    COALESCE(f.schema_type, '')           AS schema_type,
    COALESCE(f.schema_items, '')          AS schema_items,
    COALESCE(f.schema_custom, '')         AS schema_custom,
    {{ jira_field_kind('f.field_id', 'f.schema_type',
                       'f.schema_items', 'f.schema_custom') }} AS field_kind
FROM {{ source('bronze_jira', 'jira_fields') }} AS f FINAL
WHERE f.field_id IS NOT NULL

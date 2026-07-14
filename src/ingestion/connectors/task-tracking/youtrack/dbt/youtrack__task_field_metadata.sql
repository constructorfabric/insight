-- depends_on: {{ ref('youtrack__bronze_promoted') }}
{{ config(
    materialized='incremental',
    alias='youtrack__task_field_metadata',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['youtrack', 'silver:class_task_field_metadata']
) }}

-- Per-source staging projection; unioned into `silver.class_task_field_metadata`.
-- Incremental RMT (mirrors jira__task_field_metadata) so field-type observations
-- accumulate over time; `_version = _airbyte_extracted_at` is deterministic and
-- monotonic, and RMT + union_by_tag collapse re-observations to the newest row.
--
-- YouTrack custom fields are project-scoped (ADR-001), so metadata is per
-- (project, field): `project_key` resolved via a deduped join to youtrack_projects.
-- `is_multi` = fieldType.isMultiValue. `has_id`: YouTrack "typed" field values
-- carry object ids (enum/state/user/version/build/ownedField/group); scalar
-- value types (text/integer/float/date/period/string) do not.

SELECT
    pcf.unique_key                                          AS unique_key,
    pcf.source_id                                           AS insight_source_id,
    CAST('youtrack' AS String)                              AS data_source,
    pj.short_name                                           AS project_key,
    pcf.field_id                                            AS field_id,
    pcf.field_name                                          AS field_name,
    if(pcf.is_multi_value IS NULL, toUInt8(0),
       toUInt8(pcf.is_multi_value))                         AS is_multi,
    pcf.value_type                                          AS field_type,
    toUInt8(lower(toString(pcf.value_type)) NOT IN
        ('text','integer','float','date','date and time','period','string',''))  AS has_id,
    toDateTime64(pcf._airbyte_extracted_at, 3)              AS observed_at,
    toUnixTimestamp64Milli(pcf._airbyte_extracted_at)       AS _version
FROM {{ source('bronze_youtrack', 'youtrack_project_custom_fields') }} pcf
LEFT JOIN (
    SELECT source_id, project_id, short_name
    FROM {{ source('bronze_youtrack', 'youtrack_projects') }}
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY source_id, project_id
) pj ON pj.source_id = pcf.source_id AND pj.project_id = pcf.project_id

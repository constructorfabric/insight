{{ config(
    materialized='view',
    alias='github__task_issuetypes',
    schema='staging',
    tags=['github', 'staging', 'silver:class_task_issuetypes']
) }}

-- Per-source issue-type dimension; unioned into `silver.class_task_issuetypes`
-- via `union_by_tag`. GitHub states an issue's type by display name only, so
-- the organization catalogue is what gives that type a key a rename cannot
-- break. Raw vendor catalogue only: classification (the issue kind) is not
-- decided here — gold resolves it from `config.field_value_map` at its own
-- build, so a mapping change never requires a silver rebuild.

SELECT
    CAST(t.unique_key AS Nullable(String))                  AS unique_key,
    CAST(t.source_id AS Nullable(String))                   AS insight_source_id,
    CAST('github' AS String)                                AS data_source,
    CAST(t.issue_type_id AS Nullable(String))               AS issue_type_id,
    CAST(t.issue_type_name AS Nullable(String))             AS issue_type_name,
    CAST(t.issue_type_name AS Nullable(String))             AS untranslated_name,
    toDateTime64(t._airbyte_extracted_at, 3)                AS collected_at,
    toUnixTimestamp64Milli(now64(3))                        AS _version
FROM {{ source('bronze_github', 'issue_types') }} AS t FINAL

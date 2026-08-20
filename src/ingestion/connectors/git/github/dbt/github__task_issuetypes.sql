-- depends_on: {{ ref('github__bronze_promoted') }}
{{ config(
    materialized='view',
    alias='github__task_issuetypes',
    schema='staging',
    tags=['github', 'staging', 'silver:class_task_issuetypes']
) }}

-- Per-source issue-type dimension; unioned into `silver.class_task_issuetypes`
-- via `union_by_tag`. GitHub states an issue's type by display name only, so
-- the organization catalogue is what gives that type a key a rename cannot
-- break.
--
-- `issue_kind` prefers the operator's binding and falls back to the shared
-- name lists. The binding is what lets one deployment classify a type the
-- global lists never heard of without editing the lists for everybody.

WITH authored AS (
    SELECT
        tenant_id,
        insight_source_id,
        value_id,
        canonical_value
    FROM {{ source('config', 'task_value_map') }} FINAL
    WHERE data_source = 'github'
      AND field_id = 'type'
      AND is_deleted = 0
      AND valid_from <= now64(3)
    ORDER BY valid_from DESC, recorded_at DESC
    LIMIT 1 BY tenant_id, insight_source_id, value_id
)

SELECT
    CAST(t.unique_key AS Nullable(String))                  AS unique_key,
    CAST(t.source_id AS Nullable(String))                   AS insight_source_id,
    CAST('github' AS String)                                AS data_source,
    CAST(t.issue_type_id AS Nullable(String))               AS issue_type_id,
    CAST(t.issue_type_name AS Nullable(String))             AS issue_type_name,
    CAST(t.issue_type_name AS Nullable(String))             AS untranslated_name,
    -- CAST off the LowCardinality the config column carries: `union_by_tag`
    -- unions this with the other sources' branches, and one differing type
    -- fails the shared class for every source at once.
    CAST(COALESCE(
        nullIf(toString(a.canonical_value), ''),
        {{ task_issue_kind("toString(t.issue_type_name)") }}
    ) AS String)                                            AS issue_kind,
    toDateTime64(t._airbyte_extracted_at, 3)                AS collected_at,
    toUnixTimestamp64Milli(now64(3))                        AS _version
FROM {{ source('bronze_github', 'issue_types') }} AS t FINAL
LEFT JOIN authored AS a
    ON a.tenant_id = t.tenant_id
    AND a.insight_source_id = t.source_id
    AND a.value_id = t.issue_type_id

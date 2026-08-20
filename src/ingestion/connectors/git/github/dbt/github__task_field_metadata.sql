-- depends_on: {{ ref('github__bronze_promoted') }}
{{ config(
    materialized='view',
    alias='github__task_field_metadata',
    schema='staging',
    tags=['github', 'staging', 'silver:class_task_field_metadata']
) }}

-- Per-source field catalogue; unioned into `silver.class_task_field_metadata`
-- via `union_by_tag`. It states what fields exist and how their values behave,
-- never what they mean — meaning is the operator's binding in `config`.
--
-- Two row groups. The properties that live on the issue itself have no vendor
-- catalogue, so they are declared here under the names GitHub's own API uses
-- for them. The organization's native issue fields come from the catalogue
-- stream, which is also the list an operator reads when authoring a binding.
--
-- `project_key` stays null: organization issue fields are not project-scoped.
-- The column is where a board field would record its board.

WITH issue_properties AS (
    SELECT
        tenant_id,
        source_id,
        arrayJoin([
            ('state',     'state',     0, 0),
            ('assignees', 'user',      1, 1),
            ('type',      'issuetype', 0, 1),
            ('labels',    'string',    1, 0)
        ]) AS prop,
        max(_airbyte_extracted_at) AS observed_at
    FROM {{ source('bronze_github', 'repositories') }} FINAL
    GROUP BY tenant_id, source_id, prop
),

declared AS (
    SELECT
        tenant_id,
        source_id,
        prop.1 AS field_id,
        prop.1 AS field_name,
        prop.2 AS field_type,
        toUInt8(prop.3) AS is_multi,
        toUInt8(prop.4) AS has_id,
        observed_at
    FROM issue_properties
),

catalogued AS (
    SELECT
        tenant_id,
        source_id,
        COALESCE(field_id, '') AS field_id,
        COALESCE(field_name, '') AS field_name,
        lower(COALESCE(data_type, '')) AS field_type,
        toUInt8(COALESCE(is_multi, false)) AS is_multi,
        toUInt8(COALESCE(data_type, '') IN ('SINGLE_SELECT', 'MULTI_SELECT')) AS has_id,
        _airbyte_extracted_at AS observed_at
    FROM {{ source('bronze_github', 'issue_fields') }} FINAL
),

every_field AS (
    SELECT * FROM declared
    UNION ALL
    SELECT * FROM catalogued
)

SELECT
    CAST(concat(tenant_id, ':', source_id, ':github:field:', field_id) AS Nullable(String)) AS unique_key,
    CAST(COALESCE(source_id, '') AS String)                 AS insight_source_id,
    CAST('github' AS String)                                AS data_source,
    CAST(NULL AS Nullable(String))                          AS project_key,
    CAST(field_id AS String)                                AS field_id,
    CAST(field_name AS String)                              AS field_name,
    is_multi                                                AS is_multi,
    CAST(field_type AS String)                              AS field_type,
    has_id                                                  AS has_id,
    toDateTime64(observed_at, 3)                            AS observed_at,
    toUnixTimestamp64Milli(toDateTime64(observed_at, 3))    AS _version
FROM every_field

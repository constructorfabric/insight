-- depends_on: {{ ref('github__bronze_promoted') }}
{{ config(
    materialized='view',
    alias='github__task_projects',
    schema='staging',
    tags=['github', 'staging', 'silver:class_task_projects']
) }}

-- Per-source project dimension; unioned into `silver.class_task_projects` via
-- `union_by_tag`. A repository is what an issue belongs to, so a repository is
-- what plays the project's part. Gold reads none of this today; the class
-- exists so the contract is whole and a later consumer has somewhere to look.

SELECT
    CAST(unique_key AS Nullable(String))                    AS unique_key,
    CAST(source_id AS Nullable(String))                     AS insight_source_id,
    CAST('github' AS String)                                AS data_source,
    CAST(toString(id) AS Nullable(String))                  AS project_id,
    CAST(full_name AS Nullable(String))                     AS project_key,
    CAST(name AS Nullable(String))                          AS name,
    CAST(NULL AS Nullable(String))                          AS lead_id,
    CAST('repository' AS Nullable(String))                  AS project_type,
    CAST(NULL AS Nullable(String))                          AS project_style,
    CAST(if(COALESCE(archived, false), 1, 0) AS Nullable(UInt8)) AS archived,
    now64(3)                                                AS collected_at,
    toUnixTimestamp64Milli(now64(3))                        AS _version
FROM {{ source('bronze_github', 'repositories') }} FINAL

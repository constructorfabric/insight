-- depends_on: {{ ref('youtrack__bronze_promoted') }}
{{ config(
    materialized='view',
    alias='youtrack__task_projects',
    schema='staging',
    tags=['youtrack', 'silver:class_task_projects']
) }}

-- Per-source staging projection; unioned into `silver.class_task_projects`.
-- `project_type` / `project_style` have no YouTrack equivalent (Classic/Next-gen
-- is a Jira concept) — left NULL. `project_key` = YouTrack shortName.

SELECT
    p.unique_key                                            AS unique_key,
    p.source_id                                             AS insight_source_id,
    CAST('youtrack' AS String)                              AS data_source,
    toString(p.project_id)                                  AS project_id,
    p.short_name                                            AS project_key,
    p.name                                                  AS name,
    p.leader_id                                             AS lead_id,
    CAST(NULL AS Nullable(String))                          AS project_type,
    CAST(NULL AS Nullable(String))                          AS project_style,
    if(p.archived IS NULL, CAST(NULL AS Nullable(UInt8)),
       toUInt8(p.archived))                                 AS archived,
    toDateTime64(p._airbyte_extracted_at, 3)                AS collected_at,
    toUnixTimestamp64Milli(p._airbyte_extracted_at)         AS _version
FROM {{ source('bronze_youtrack', 'youtrack_projects') }} p

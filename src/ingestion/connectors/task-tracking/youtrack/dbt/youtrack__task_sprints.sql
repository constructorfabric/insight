-- depends_on: {{ ref('youtrack__bronze_promoted') }}
{{ config(
    materialized='view',
    alias='youtrack__task_sprints',
    schema='staging',
    tags=['youtrack', 'silver:class_task_sprints']
) }}

-- Per-source staging projection; unioned into `silver.class_task_sprints`.
-- YouTrack sprints belong to agile boards, which may span multiple projects, so
-- `project_key` is left NULL (same as Jira). `board_name` is available via
-- youtrack_agiles.name but left NULL here to match the Jira projection; join
-- youtrack_agiles later if a consumer needs it. YouTrack has no explicit sprint
-- state or a distinct "completed" timestamp — `state` is derived from `archived`,
-- `complete_date` is NULL. Timestamps are epoch-ms integers in bronze.

SELECT
    s.unique_key                                            AS unique_key,
    s.source_id                                             AS insight_source_id,
    CAST('youtrack' AS String)                              AS data_source,
    toString(s.sprint_id)                                   AS sprint_id,
    toString(s.agile_id)                                    AS board_id,
    CAST(NULL AS Nullable(String))                          AS board_name,
    s.sprint_name                                           AS sprint_name,
    CAST(NULL AS Nullable(String))                          AS project_key,
    multiIf(s.archived IS NULL, CAST(NULL AS Nullable(String)),
            s.archived, 'closed', 'active')                 AS state,
    fromUnixTimestamp64Milli(toInt64OrNull(toString(s.start_date)))  AS start_date,
    fromUnixTimestamp64Milli(toInt64OrNull(toString(s.finish_date))) AS end_date,
    CAST(NULL AS Nullable(DateTime64(3)))                   AS complete_date,
    toDateTime64(s._airbyte_extracted_at, 3)                AS collected_at,
    toUnixTimestamp64Milli(s._airbyte_extracted_at)         AS _version
FROM {{ source('bronze_youtrack', 'youtrack_sprints') }} s

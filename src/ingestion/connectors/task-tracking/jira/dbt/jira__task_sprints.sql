-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='view',
    alias='jira__task_sprints',
    schema='staging',
    tags=['jira', 'silver:class_task_sprints']
) }}

-- View, not table: bronze `jira_sprints` is MergeTree (full_refresh + overwrite),
-- so the current state of bronze is the current state of staging — no incremental
-- accumulation needed. Silver `class_task_sprints` is RMT(_version), reads via FINAL.

-- Bronze `jira_sprints` doesn't carry `board_name` or `project_key` (Phase 1 SubstreamPartitionRouter
-- limitation per jira/jira.md). Left NULL here.

SELECT
    s.unique_key                                AS unique_key,
    s.source_id                                 AS insight_source_id,
    CAST('jira' AS String)                      AS data_source,
    toString(s.sprint_id)                       AS sprint_id,
    toString(s.board_id)                        AS board_id,
    CAST(NULL AS Nullable(String))              AS board_name,
    s.sprint_name                               AS sprint_name,
    CAST(NULL AS Nullable(String))              AS project_key,
    s.state                                     AS state,
    parseDateTime64BestEffortOrNull(s.start_date, 3)     AS start_date,
    parseDateTime64BestEffortOrNull(s.end_date, 3)       AS end_date,
    parseDateTime64BestEffortOrNull(s.complete_date, 3)  AS complete_date,
    now64(3)                                    AS collected_at,
    toUnixTimestamp64Milli(now64(3))            AS _version
FROM {{ source('bronze_jira', 'jira_sprints') }} s
-- `jira_sprints` bronze = MergeTree (full_refresh + overwrite), FINAL not supported and not needed.

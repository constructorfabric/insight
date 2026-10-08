{{ config(
    materialized='incremental',
    alias='jira__task_worklogs',
    incremental_strategy='append',
    on_schema_change='append_new_columns',
    schema='staging',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['jira', 'staging', 'silver:class_task_worklogs']
) }}

-- State (including is_deleted — specs/DELETION-AND-VISIBILITY.md) is computed
-- once in jira__worklog_state; this is the class-contract projection.

SELECT
    s.unique_key                                        AS unique_key,
    s.source_id                                         AS insight_source_id,
    CAST('jira' AS String)                              AS data_source,
    s.worklog_id                                        AS worklog_id,
    s.id_readable                                       AS id_readable,
    s.author_id                                         AS author_id,
    s.work_date                                         AS work_date,
    s.duration_seconds                                  AS duration_seconds,
    s.description                                       AS description,
    s.collected_at                                      AS collected_at,
    toNullable(s.is_deleted)                            AS is_deleted,
    toUnixTimestamp64Milli(now64(3))                    AS _version,
    CAST(p.issue_id AS Nullable(String))                 AS issue_id
FROM {{ ref('jira__worklog_state') }} AS s FINAL
LEFT JOIN (
    SELECT tenant_id, source_id, toString(worklog_id) AS worklog_id,
        argMax(jira_id, _airbyte_extracted_at) AS issue_id
    FROM {{ source('bronze_jira', 'jira_worklogs') }} FINAL
    GROUP BY tenant_id, source_id, worklog_id
) AS p ON p.tenant_id = s.tenant_id AND p.source_id = s.source_id AND p.worklog_id = s.worklog_id

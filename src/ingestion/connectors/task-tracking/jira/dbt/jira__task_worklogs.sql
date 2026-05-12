-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='incremental',
    alias='jira__task_worklogs',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['jira', 'silver:class_task_worklogs']
) }}

SELECT
    w.unique_key                                      AS unique_key,
    w.source_id                                       AS insight_source_id,
    CAST('jira' AS String)                            AS data_source,
    w.worklog_id                                      AS worklog_id,
    w.id_readable                                     AS id_readable,
    w.author_account_id                               AS author_id,
    parseDateTime64BestEffortOrNull(w.started, 3)     AS work_date,
    toFloat64OrNull(toString(w.time_spent_seconds))   AS duration_seconds,
    w.comment                                         AS description,
    parseDateTime64BestEffortOrNull(w.collected_at, 3) AS collected_at,
    toUnixTimestamp64Milli(now64(3))                  AS _version
FROM (
    SELECT * FROM {{ source('bronze_jira', 'jira_worklogs') }}
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY _airbyte_raw_id
) w

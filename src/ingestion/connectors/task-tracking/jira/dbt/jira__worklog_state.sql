-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='table',
    schema='staging',
    engine='ReplacingMergeTree',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['jira', 'staging'],
    query_settings={'join_use_nulls': 1}
) }}

-- Current state per worklog, deletion included (specs/DELETION-AND-VISIBILITY.md).
-- is_deleted, three signals OR-ed: an authoritative /worklog/deleted tombstone;
-- the re-fetch generation diff (editing/deleting a worklog bumps the issue's
-- `updated`, re-syncing its full worklog list); a deleted/trashed parent
-- issue. Feeds both the class projection (jira__task_worklogs) and the
-- lifecycle snapshot chain.

WITH worklogs AS (
    SELECT *
    FROM {{ source('bronze_jira', 'jira_worklogs') }}
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY unique_key
),

tombstones AS (
    SELECT
        tenant_id,
        source_id,
        toString(toInt64(worklog_id))                   AS worklog_id,
        max(toInt64(deleted_at_ms))                     AS deleted_at_ms
    FROM {{ source('bronze_jira', 'jira_worklog_deleted') }} FINAL
    GROUP BY tenant_id, source_id, worklog_id
),

issue_fetch AS (
    SELECT
        tenant_id,
        source_id,
        id_readable,
        max(_airbyte_extracted_at)                      AS last_fetched_at
    FROM {{ source('bronze_jira', 'jira_issue_keys') }} FINAL
    GROUP BY tenant_id, source_id, id_readable
),

unavailable_issues AS (
    SELECT
        tenant_id,
        source_id,
        id_readable
    FROM {{ ref('jira__issue_availability_state') }} FINAL
    WHERE availability IN ('deleted', 'trashed')
      AND id_readable IS NOT NULL
)

SELECT
    w.unique_key                                        AS unique_key,
    w.tenant_id                                         AS tenant_id,
    w.source_id                                         AS source_id,
    toString(w.worklog_id)                              AS worklog_id,
    w.id_readable                                       AS id_readable,
    w.author_account_id                                 AS author_id,
    parseDateTime64BestEffortOrNull(w.started, 3)       AS work_date,
    toFloat64OrNull(toString(w.time_spent_seconds))     AS duration_seconds,
    w.comment                                           AS description,
    parseDateTime64BestEffortOrNull(w.created, 3)       AS created_at,
    parseDateTime64BestEffortOrNull(w.updated, 3)       AS edited_at,
    parseDateTime64BestEffortOrNull(w.collected_at, 3)  AS collected_at,
    toUInt8(
        ts.worklog_id IS NOT NULL
        OR (f.last_fetched_at IS NOT NULL
            AND w._airbyte_extracted_at < f.last_fetched_at
                - INTERVAL {{ var('jira_comment_refetch_tolerance_hours', 6) }} HOUR)
        OR ui.id_readable IS NOT NULL
    )                                                   AS is_deleted,
    -- Real deletion time when the tombstone carries it; NULL otherwise
    -- (generation-diff / parent-issue deletions are dated at detection).
    ts.deleted_at_ms                                    AS deleted_at_ms
FROM worklogs AS w
LEFT JOIN tombstones AS ts
    ON ts.tenant_id = w.tenant_id
    AND ts.source_id = w.source_id
    AND ts.worklog_id = toString(w.worklog_id)
LEFT JOIN issue_fetch AS f
    ON f.tenant_id = w.tenant_id
    AND f.source_id = w.source_id
    AND f.id_readable = w.id_readable
LEFT JOIN unavailable_issues AS ui
    ON ui.tenant_id = w.tenant_id
    AND ui.source_id = w.source_id
    AND ui.id_readable = w.id_readable

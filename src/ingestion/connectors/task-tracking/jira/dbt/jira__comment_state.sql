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

-- Current state per comment, deletion included (specs/DELETION-AND-VISIBILITY.md).
-- is_deleted: deleting a comment bumps the issue's `updated`, which re-syncs
-- the issue's FULL comment list — a comment whose last observation predates
-- its issue's last re-fetch was deleted at the source; comments of
-- deleted/trashed issues are marked through the availability state (their
-- list is never re-fetched). Feeds both the class projection
-- (jira__task_comments) and the lifecycle snapshot chain.

WITH comments AS (
    SELECT *
    FROM {{ source('bronze_jira', 'jira_comments') }}
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY unique_key
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
    c.unique_key                                        AS unique_key,
    c.tenant_id                                         AS tenant_id,
    c.source_id                                         AS source_id,
    toString(c.comment_id)                              AS comment_id,
    c.id_readable                                       AS id_readable,
    c.author_account_id                                 AS author_id,
    parseDateTime64BestEffortOrNull(c.created, 3)       AS created_at,
    parseDateTime64BestEffortOrNull(c.updated, 3)       AS edited_at,
    c.body                                              AS body,
    toUInt8(
        (f.last_fetched_at IS NOT NULL
         AND c._airbyte_extracted_at < f.last_fetched_at
             - INTERVAL {{ var('jira_comment_refetch_tolerance_hours', 6) }} HOUR)
        OR ui.id_readable IS NOT NULL
    )                                                   AS is_deleted
FROM comments AS c
LEFT JOIN issue_fetch AS f
    ON f.tenant_id = c.tenant_id
    AND f.source_id = c.source_id
    AND f.id_readable = c.id_readable
LEFT JOIN unavailable_issues AS ui
    ON ui.tenant_id = c.tenant_id
    AND ui.source_id = c.source_id
    AND ui.id_readable = c.id_readable

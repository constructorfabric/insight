{{ config(
    materialized='table',
    schema='staging',
    engine=insight_engine('ReplacingMergeTree'),
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['jira', 'staging'],
    query_settings={'join_use_nulls': 1}
) }}

-- Current state per comment, deletion included (specs/DELETION-AND-VISIBILITY.md).
-- is_deleted: every read of an issue's comment list re-emits each comment still
-- on it, so a comment last emitted before its issue's newest comment read was
-- deleted at the source; comments of deleted/trashed issues are marked through
-- the availability state. Feeds both the class projection (jira__task_comments)
-- and the lifecycle snapshot chain.

WITH comments AS (
    SELECT * REPLACE (CAST(tenant_id AS String) AS tenant_id)
    FROM {{ source('bronze_jira', 'jira_comments') }}
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY unique_key
),

-- INVARIANT: only a read of the same list is evidence, and it holds only while
-- jira_comments re-emits the whole list (no client-side cursor filter).
issue_comment_reads AS (
    SELECT
        CAST(tenant_id AS String)                       AS tenant_id,
        source_id,
        jira_id,
        max(_airbyte_extracted_at)                      AS last_read_at
    FROM comments
    WHERE jira_id IS NOT NULL
    GROUP BY tenant_id, source_id, jira_id
),

unavailable_issues AS (
    SELECT
        tenant_id,
        source_id,
        jira_id
    FROM {{ ref('jira__issue_availability_state') }} FINAL
    WHERE availability IN ('deleted', 'trashed')
      AND jira_id IS NOT NULL
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
        (r.last_read_at IS NOT NULL
         AND c._airbyte_extracted_at < r.last_read_at
             - INTERVAL {{ var('jira_comment_refetch_tolerance_hours', 1) }} HOUR)
        OR ui.jira_id IS NOT NULL
    )                                                   AS is_deleted
FROM comments AS c
LEFT JOIN issue_comment_reads AS r
    ON r.tenant_id = c.tenant_id
    AND r.source_id = c.source_id
    AND r.jira_id = c.jira_id
LEFT JOIN unavailable_issues AS ui
    ON ui.tenant_id = c.tenant_id
    AND ui.source_id = c.source_id
    AND ui.jira_id = c.jira_id

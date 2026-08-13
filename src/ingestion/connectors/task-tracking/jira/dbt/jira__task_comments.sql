-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='incremental',
    alias='jira__task_comments',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['jira', 'staging', 'silver:class_task_comments'],
    query_settings={'join_use_nulls': 1}
) }}

-- `body` is raw ADF JSON at Bronze level; plaintext extraction deferred.
--
-- is_deleted (specs/DELETION-AND-VISIBILITY.md): deleting a comment bumps the
-- issue's `updated`, which re-syncs the issue's FULL comment list — so a
-- comment whose last observation predates its issue's last re-fetch was
-- deleted at the source. The tolerance absorbs the lag between the parent
-- fetch and the comment fetch inside one sync. Comments of issues that are
-- themselves deleted/trashed are marked deleted through the availability
-- state (their comment list is never re-fetched, so the generation
-- comparison alone would miss them).

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
    c.source_id                                         AS insight_source_id,
    CAST('jira' AS String)                              AS data_source,
    toString(c.comment_id)                              AS comment_id,
    c.id_readable                                       AS id_readable,
    c.author_account_id                                 AS author_id,
    parseDateTime64BestEffortOrNull(c.created, 3)       AS created_at,
    parseDateTime64BestEffortOrNull(c.updated, 3)       AS updated_at,
    c.body                                              AS body,
    toNullable(toUInt8(
        (f.last_fetched_at IS NOT NULL
         AND c._airbyte_extracted_at < f.last_fetched_at
             - INTERVAL {{ var('jira_comment_refetch_tolerance_hours', 6) }} HOUR)
        OR ui.id_readable IS NOT NULL
    ))                                                  AS is_deleted,
    toUnixTimestamp64Milli(now64(3))                    AS _version
FROM comments AS c
LEFT JOIN issue_fetch AS f
    ON f.tenant_id = c.tenant_id
    AND f.source_id = c.source_id
    AND f.id_readable = c.id_readable
LEFT JOIN unavailable_issues AS ui
    ON ui.tenant_id = c.tenant_id
    AND ui.source_id = c.source_id
    AND ui.id_readable = c.id_readable

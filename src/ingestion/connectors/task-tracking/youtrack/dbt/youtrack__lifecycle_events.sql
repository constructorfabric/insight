{{ config(materialized='view', schema='staging', tags=['youtrack', 'staging']) }}

-- One event per observed state, not per observation: work items are re-read on
-- every sync and a comment whenever its issue changes, so an unchanged record
-- is observed again and again. A state is identified by what it says — a
-- comment's (deleted, updated), a work item's updated — and dated and keyed by
-- its first observation, so a later re-read neither adds an event nor moves a
-- deletion's time forward.
SELECT insight_source_id, issue_id,
    concat('comment:', comment_id, ':', toString(first_seen)) AS event_id,
    if(is_deleted, first_seen, coalesce(changed_at, first_seen)) AS event_at,
    author_id, 'comment' AS field_id,
    if(is_deleted, 'remove', 'set') AS delta_action,
    [(comment_id, comment_id)] AS pairs,
    first_seen AS collected_at
FROM (
    SELECT coalesce(source_id, '') AS insight_source_id, coalesce(issue_id, '') AS issue_id,
        coalesce(id, '') AS comment_id, ifNull(deleted, false) AS is_deleted,
        coalesce({{ youtrack_timestamp('updated') }}, fromUnixTimestamp64Milli(toInt64(created))) AS changed_at,
        argMin(author_id, _airbyte_extracted_at) AS author_id,
        min(toDateTime64(_airbyte_extracted_at, 3)) AS first_seen
    FROM {{ ref('youtrack__comment_observations') }} FINAL
    GROUP BY insight_source_id, issue_id, comment_id, is_deleted, changed_at
)
UNION ALL
SELECT insight_source_id, issue_id,
    concat('worklog:', work_item_id, ':', toString(first_seen)),
    coalesce(changed_at, first_seen),
    author_id, 'worklog', 'set', [(work_item_id, work_item_id)], first_seen
FROM (
    SELECT coalesce(source_id, '') AS insight_source_id, coalesce(issue_id, '') AS issue_id,
        coalesce(id, '') AS work_item_id,
        coalesce({{ youtrack_timestamp('updated') }}, fromUnixTimestamp64Milli(toInt64(created))) AS changed_at,
        argMin(author_id, _airbyte_extracted_at) AS author_id,
        min(toDateTime64(_airbyte_extracted_at, 3)) AS first_seen
    FROM {{ ref('youtrack__work_item_observations') }} FINAL
    GROUP BY insight_source_id, issue_id, work_item_id, changed_at
)

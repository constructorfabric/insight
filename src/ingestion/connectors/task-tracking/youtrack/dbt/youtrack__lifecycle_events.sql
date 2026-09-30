{{ config(materialized='view', schema='staging', tags=['youtrack', 'staging']) }}

SELECT coalesce(c.source_id, '') AS insight_source_id, coalesce(c.issue_id, '') AS issue_id,
    concat('comment:', coalesce(c.id, ''), ':', toString(c._airbyte_extracted_at)) AS event_id,
    if(c.deleted = true, toDateTime64(c._airbyte_extracted_at, 3),
       coalesce({{ youtrack_timestamp('c.updated') }}, fromUnixTimestamp64Milli(toInt64(c.created)), toDateTime64(c._airbyte_extracted_at, 3))) AS event_at,
    c.author_id AS author_id, 'comment' AS field_id,
    if(c.deleted = true, 'remove', 'set') AS delta_action,
    [(coalesce(c.id, ''), coalesce(c.id, ''))] AS pairs,
    toDateTime64(c._airbyte_extracted_at, 3) AS collected_at
FROM {{ ref('youtrack__comment_observations') }} AS c FINAL
UNION ALL
SELECT coalesce(source_id, ''), coalesce(issue_id, ''),
    concat('worklog:', coalesce(id, ''), ':', toString(_airbyte_extracted_at)),
    coalesce({{ youtrack_timestamp('updated') }}, fromUnixTimestamp64Milli(toInt64(created)), toDateTime64(_airbyte_extracted_at, 3)),
    author_id, 'worklog', 'set', [(coalesce(id, ''), coalesce(id, ''))], toDateTime64(_airbyte_extracted_at, 3)
FROM {{ ref('youtrack__work_item_observations') }} FINAL

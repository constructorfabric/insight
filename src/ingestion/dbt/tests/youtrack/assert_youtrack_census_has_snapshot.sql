-- Every census issue the snapshot streams are meant to cover has a snapshot.
-- They cover the collection window: an issue last updated before
-- youtrack_start_date is in the census and deliberately never snapshotted, and
-- one updated after the search last read was created or changed mid-sync and is
-- the next sync's. The window is read from youtrack_issues, the `updated:`
-- search that defines it.
WITH collection_window AS (
    SELECT source_id,
        min(toInt64(assumeNotNull(updated))) AS window_start,
        toUnixTimestamp64Milli(max(_airbyte_extracted_at)) AS last_read
    FROM {{ source('bronze_youtrack', 'youtrack_issues') }}
    WHERE updated IS NOT NULL
    GROUP BY source_id
)
SELECT c.source_id, c.id
FROM {{ source('bronze_youtrack', 'youtrack_issue_census') }} AS c FINAL
INNER JOIN collection_window AS w ON w.source_id = c.source_id
LEFT ANTI JOIN {{ ref('youtrack__issues') }} AS i
    ON i.insight_source_id = c.source_id AND i.issue_id = c.id
WHERE toInt64(assumeNotNull(c.updated)) >= w.window_start
  AND toInt64(assumeNotNull(c.updated)) < w.last_read

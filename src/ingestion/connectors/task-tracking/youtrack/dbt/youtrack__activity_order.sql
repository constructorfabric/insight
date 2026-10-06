{{ config(materialized='table', schema='staging', engine='MergeTree', order_by=['insight_source_id', 'issue_id', 'event_id'], tags=['youtrack', 'staging']) }}

WITH groups AS (
    SELECT insight_source_id, issue_id, event_at,
        arraySort(x -> x.1, groupArray((event_id, field_id, before_pairs, after_pairs))) AS entries
    FROM {{ ref('youtrack__activities') }}
    WHERE restated = 0
    GROUP BY insight_source_id, issue_id, event_at
), edges AS (
    SELECT *, arrayFlatten(arrayMap(a -> arrayMap(b -> (a, b),
        arrayFilter(b -> a != b AND entries[a].2 = entries[b].2
            AND length(entries[a].4) > 0 AND entries[a].4 = entries[b].3,
            range(1, length(entries) + 1))), range(1, length(entries) + 1))) AS links
    FROM groups
), ordered AS (
    SELECT *, {{ task_instant_order('length(entries)', 'links') }} AS positions
    FROM edges
)
SELECT insight_source_id, issue_id, event_at, entries[p].1 AS event_id,
    toUInt32(indexOf(positions, p) - 1) AS entry_rank
FROM ordered ARRAY JOIN positions AS p
WHERE throwIf(length(entries) >= 100000, 'YouTrack event_order rank overflow') = 0

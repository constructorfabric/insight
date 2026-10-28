{{ config(materialized='view', schema='staging', tags=['youtrack', 'staging']) }}

-- The issue each comment belongs to, as the issue snapshots list their comments.
SELECT insight_source_id, comment.1 AS comment_id, argMax(issue_id, observed_at) AS issue_id
FROM {{ ref('youtrack__issues') }}
ARRAY JOIN JSONExtract(payload, 'comments', 'Array(Tuple(id String))') AS comment
GROUP BY insight_source_id, comment_id

-- After ReplacingMergeTree merge there must be at most one row per primary key.
-- (FINAL forces merge — if >1 row appears, our key is wrong or writes race something we missed.)
-- Grouping mirrors the silver ORDER BY: (insight_source_id, data_source, issue_id, field_id, event_id).
-- The issue is keyed by `issue_id`, not by the readable key, which changes when
-- a repository is renamed or an issue moves between projects (#2741). Rows
-- written under the previous key formula stay until the class is rebuilt, so
-- this reports a moved issue's history twice until the next full refresh.

SELECT
    insight_source_id,
    data_source,
    issue_id,
    field_id,
    event_id,
    count() AS n
FROM silver.class_task_field_history FINAL
GROUP BY insight_source_id, data_source, issue_id, field_id, event_id
HAVING n > 1
LIMIT 100

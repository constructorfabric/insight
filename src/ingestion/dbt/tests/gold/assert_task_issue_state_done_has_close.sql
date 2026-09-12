-- Build-integrity check (untagged → error severity under `dbt build`).
-- An issue whose CURRENT status category is 'done' closed at its last
-- transition into a done-category status, so `final_close_at` must be set:
-- the current-status join and the close scan read the same status history
-- against the same status dimension, and a NULL here means they disagree —
-- the closed-issue measures gated on `final_close_at IS NOT NULL` would
-- silently drop an issue the state table itself calls done.
SELECT
    insight_source_id,
    issue_id,
    id_readable,
    status_category,
    final_close_at
FROM {{ ref('task_issue_state') }}
WHERE status_category = 'done'
  AND final_close_at IS NULL
LIMIT 100

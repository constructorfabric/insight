-- Changelog rows must use a delta_action consistent with field cardinality:
--   single  → set
--   multi   → add | remove | set (set allowed for Snapshot semantics on legacy list fields)
-- Initial rows use set (single) or add (multi) — this test only covers changelog kind.

SELECT
    insight_source_id,
    data_source,
    issue_id,
    field_id,
    field_cardinality,
    delta_action,
    event_id
FROM silver.class_task_field_history FINAL
WHERE event_kind = 'changelog'
  AND (
       (field_cardinality = 'single' AND delta_action NOT IN ('set'))
    OR (field_cardinality = 'multi'  AND delta_action NOT IN ('set', 'add', 'remove'))
  )
LIMIT 100

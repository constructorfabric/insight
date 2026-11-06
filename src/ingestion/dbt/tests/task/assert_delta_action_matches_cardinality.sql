{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'Changelog delta actions fit the field cardinality',
        'domain': 'task-tracking',
        'category': 'consistency',
        'tier': 'error',
        'remediation': 'A single-value field carries an add or remove event, or a multi-value field carries an action other than set, add or remove. The producer named by data_source in the finding emits the wrong action for the field\'s cardinality.'
    }
) }}

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

{{ config(
    severity='warn',
    tags=['connector_quality', 'jira'],
    store_failures=true,
    meta={
        'title': 'A Jira changelog entry carries one item per self-describing field',
        'domain': 'task-tracking',
        'category': 'consistency',
        'tier': 'warn',
        'remediation': 'An item of a self-describing kind states the field\'s whole value, so an entry is expected to carry one per field. The journal collapses several into one event (FIELD-HISTORY-IN-DBT.md §5), which is exact when the items chain from one value to the next and a best effort when they do not. Items that each add or remove one element mean the field changes element by element: compare its `schema_type` / `schema_items` / `schema_custom` with the kind rules in §3 — an element-wise field classified as a full-list kind keeps a wrong history between events, and only its final state is repaired by `snapshot_diff`.'
    }
) }}

-- Entries the journal had to collapse, per field, so a field whose changelog
-- does not match its kind is visible rather than absorbed.

SELECT
    insight_source_id,
    field_id,
    any(field_kind)                                       AS field_kind,
    count()                                               AS entries,
    groupArray(10)(changelog_id)                          AS sample_changelog_ids
FROM (
    SELECT
        ci.insight_source_id                              AS insight_source_id,
        ci.changelog_id                                   AS changelog_id,
        ci.field_id                                       AS field_id,
        any(k.field_kind)                                 AS field_kind
    FROM {{ ref('jira__changelog_items') }} AS ci FINAL
    INNER JOIN {{ ref('jira__task_field_kind') }} AS k
        ON k.insight_source_id = ci.insight_source_id
       AND k.field_id = ci.field_id
    WHERE ci.jira_id IS NOT NULL
      AND ci.field_id != 'created'
      AND k.field_kind NOT IN ('ignored', 'UNKNOWN')
      AND k.field_kind NOT IN {{ jira_element_wise_kinds() }}
      AND {{ jira_item_is_live('k.field_kind', 'ci.value_from', 'ci.value_from_string',
                               'ci.value_to', 'ci.value_to_string') }}
    GROUP BY ci.insight_source_id, ci.changelog_id, ci.field_id
    HAVING count() > 1
)
GROUP BY insight_source_id, field_id

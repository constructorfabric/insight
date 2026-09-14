{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'Jira issue-type mappings still name the type they were decided on',
        'domain': 'task',
        'category': 'freshness',
        'tier': 'warn',
        'remediation': 'The Jira issue type was renamed after the operator classified it. `issue_kind` is keyed on the type id, so the classification still holds and no number moved — but the decision was made about a type called something else and may no longer be the intent. Review the mapping, then set value_display to the current name (a new task_value_map row with a later recorded_at) to clear the finding.'
    }
) }}
-- Jira classification is id-keyed, so a rename cannot re-bucket history. That
-- makes a rename invisible, which is the point of this check: `value_display`
-- is the name the decision was recorded against, and a drift from the current
-- catalog name is the only signal an operator gets.

WITH authored AS (
    SELECT
        tenant_id,
        insight_source_id,
        value_id,
        argMax(canonical_value, (valid_from, recorded_at)) AS canonical_value,
        argMax(value_display, (valid_from, recorded_at))   AS value_display,
        argMax(is_deleted, (valid_from, recorded_at))      AS is_deleted
    FROM config.task_value_map FINAL
    WHERE data_source = 'jira'
      AND field_id = 'type'
      AND valid_from <= now64(3)
    GROUP BY tenant_id, insight_source_id, value_id
    HAVING is_deleted = 0 AND value_display != ''
)

SELECT
    a.tenant_id         AS tenant_id,
    a.insight_source_id AS insight_source_id,
    a.value_id          AS issue_type_id,
    a.value_display     AS decided_against_name,
    t.issue_type_name   AS catalog_name,
    t.untranslated_name AS catalog_untranslated_name,
    a.canonical_value   AS issue_kind
FROM authored AS a
INNER JOIN {{ ref('class_task_issuetypes') }} AS t FINAL
    ON t.insight_source_id = a.insight_source_id
    AND t.data_source = 'jira'
    AND t.issue_type_id = a.value_id
-- Either name clears the row: an operator records whichever the console showed.
WHERE lower(trimBoth(a.value_display)) != lower(trimBoth(t.issue_type_name))
  AND lower(trimBoth(a.value_display)) != lower(trimBoth(ifNull(t.untranslated_name, '')))

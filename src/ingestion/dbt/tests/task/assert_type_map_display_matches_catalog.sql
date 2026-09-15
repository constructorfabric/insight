{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'Issue-type decisions still name the type they were decided on',
        'domain': 'task',
        'category': 'freshness',
        'tier': 'warn',
        'remediation': 'The vendor issue type was renamed after the operator classified it. The kind is keyed on the type id, so the classification still holds and no number moved — but the decision was made about a type called something else and may no longer be the intent. Review the decision, then set display_name to the current name (a new config.field_value_map row with a later recorded_at) to clear the finding.'
    }
) }}
-- Classification is id-keyed, so a rename cannot re-bucket history. That makes
-- a rename invisible, which is the point of this check: `display_name` is the
-- name the decision was recorded against, and a drift from the current
-- catalogue name is the only signal an operator gets.

WITH authored AS (
    SELECT
        tenant_id,
        insight_source_id,
        data_source,
        source_key,
        argMax(target_value, (valid_from, recorded_at)) AS target_value,
        argMax(display_name, (valid_from, recorded_at)) AS display_name,
        argMax(is_deleted, (valid_from, recorded_at))   AS is_deleted
    FROM config.field_value_map FINAL
    WHERE field = 'issue_type'
      AND valid_from <= now64(3)
    GROUP BY tenant_id, insight_source_id, data_source, source_key
    HAVING is_deleted = 0 AND display_name != ''
)

SELECT
    a.tenant_id         AS tenant_id,
    a.insight_source_id AS insight_source_id,
    a.data_source       AS data_source,
    a.source_key        AS source_key,
    a.display_name      AS decided_against_name,
    t.issue_type_name   AS catalog_name,
    t.untranslated_name AS catalog_untranslated_name,
    a.target_value      AS issue_kind
FROM authored AS a
INNER JOIN {{ ref('class_task_issuetypes') }} AS t FINAL
    ON t.insight_source_id = a.insight_source_id
    AND t.data_source = a.data_source
    AND t.issue_type_id = a.source_key
-- Either name clears the row: an operator records whichever the console showed.
WHERE lower(trimBoth(a.display_name)) != lower(trimBoth(t.issue_type_name))
  AND lower(trimBoth(a.display_name)) != lower(trimBoth(ifNull(t.untranslated_name, '')))
LIMIT 100

-- A text reference — in the history, or a current body — must name a stored
-- body of its own tenant's source, and a history display must be a prefix of that body.
-- An unresolved reference leaves a long-text value with only its first
-- characters readable, or none.

WITH history_refs AS (
    SELECT
        tenant_id,
        insight_source_id,
        data_source,
        issue_id,
        field_id,
        event_id,
        text_id,
        display
    FROM {{ ref('class_task_field_history') }} FINAL
    ARRAY JOIN
        value_ids AS text_id,
        value_displays AS display
    WHERE value_id_type = {{ task_text_ref_type() }}
),

current_refs AS (
    SELECT
        tenant_id,
        insight_source_id,
        data_source,
        issue_id,
        field_id,
        'current' AS event_id,
        text_id,
        '' AS display
    FROM {{ ref('class_task_field_current_text') }} FINAL
),

text_refs AS (
    SELECT * FROM history_refs
    UNION ALL
    SELECT * FROM current_refs
)

SELECT
    r.tenant_id,
    r.insight_source_id,
    r.data_source,
    r.issue_id,
    r.field_id,
    r.event_id,
    r.text_id,
    if(ifNull(t.text_id, '') = '', 'no stored body', 'display is not a prefix of the body') AS finding
FROM text_refs AS r
LEFT JOIN {{ ref('class_task_field_text') }} AS t FINAL
    ON t.tenant_id = r.tenant_id
   AND t.insight_source_id = r.insight_source_id
   AND t.data_source = r.data_source
   AND t.text_id = r.text_id
WHERE ifNull(t.text_id, '') = '' OR NOT startsWith(t.content, r.display)
LIMIT 100

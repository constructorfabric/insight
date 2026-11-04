{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'Long-text references resolve to a stored body',
        'domain': 'task-tracking',
        'category': 'integrity',
        'tier': 'warn',
        'remediation': 'A text reference in the field history or the current-text class points at no stored body, or its display is not a prefix of that body. A run that failed between the history and the text class shows here until the next run; otherwise the text id formula drifted.'
    }
) }}

-- A text reference — in the history, or a current body — must name a stored
-- body of its own source, and a history display must be a prefix of that body.
-- An unresolved reference leaves a long-text value with only its first
-- characters readable, or none.

WITH history_refs AS (
    SELECT
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
      -- INVARIANT: ARRAY JOIN errors on arrays of unequal length; those rows are
      -- assert_value_arrays_same_length's finding, not this one's.
      AND length(value_ids) = length(value_displays)
),

current_refs AS (
    SELECT
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
    r.insight_source_id,
    r.data_source,
    r.issue_id,
    r.field_id,
    r.event_id,
    r.text_id,
    if(ifNull(t.text_id, '') = '', 'no stored body', 'display is not a prefix of the body') AS finding
FROM text_refs AS r
LEFT JOIN {{ ref('class_task_field_text') }} AS t FINAL
    ON t.insight_source_id = r.insight_source_id
   AND t.data_source = r.data_source
   AND t.text_id = r.text_id
WHERE ifNull(t.text_id, '') = '' OR NOT startsWith(t.content, r.display)

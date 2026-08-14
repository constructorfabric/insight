-- Convention (ADR-005-event-id-traceability):
--   event_kind='synthetic_initial' ⇔ event_id LIKE 'initial:%'
--   event_kind='availability'      ⇔ event_id LIKE 'availability:%'
--   event_kind='lifecycle'         ⇔ event_id LIKE 'comment:%' OR 'worklog:%'
--   event_kind='changelog'         ⇔ event_id matches no prefix above
-- Any violation means the writer got the kind vs id wrong.

SELECT
    insight_source_id,
    data_source,
    issue_id,
    field_id,
    event_id,
    event_kind
FROM silver.class_task_field_history FINAL
WHERE (event_kind = 'synthetic_initial' AND event_id NOT LIKE 'initial:%')
   OR (event_kind = 'changelog'         AND (event_id LIKE 'initial:%' OR event_id LIKE 'availability:%'
                                             OR event_id LIKE 'comment:%' OR event_id LIKE 'worklog:%'))
   OR (event_kind = 'availability'      AND event_id NOT LIKE 'availability:%')
   OR (event_kind = 'lifecycle'         AND event_id NOT LIKE 'comment:%' AND event_id NOT LIKE 'worklog:%')
LIMIT 100

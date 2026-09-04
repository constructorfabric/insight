-- No field in the catalogue may resolve to UNKNOWN.
--
-- UNKNOWN means nobody has classified this structure yet, as distinct from
-- `ignored`, which means somebody looked and decided (FIELD-HISTORY-IN-DBT.md
-- §3.1). Reaching it is the loud failure the design asks for: the field's value
-- would otherwise be normalized by a rule that was never written for it.
--
-- Onboarding another Jira instance is the expected way to trip this — an
-- installed app can present a field type this repository has not seen. The fix
-- is a classifier branch or an override row, not a widened default.

SELECT
    insight_source_id,
    field_id,
    field_name,
    schema_type,
    schema_items,
    schema_custom
FROM {{ ref('jira__task_field_kind') }}
WHERE field_kind = 'UNKNOWN'
LIMIT 100

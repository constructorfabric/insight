-- A role names one field. Every consumer joins `task_field_roles_current` by
-- role and takes what comes back, so two fields bound to the same role in one
-- source do not compete — they MERGE. For `status` that means two independent
-- timelines interleaved into one ordered event array, and a span sequence that
-- belongs to neither field.
--
-- This is reachable the moment boards are bindable: an issue's own open/closed
-- lifecycle and a Projects V2 board column are different fields, and both look
-- like a status. `config.task_field_roles.precedence` exists to choose between
-- them and is read by nothing yet, so until it is, binding both is a defect
-- rather than a configuration.
--
-- `ignored` is excluded: it is the explicit "somebody looked and decided not
-- to use this field" marker, and any number of fields may carry it.

WITH bound AS (
    SELECT
        insight_source_id,
        data_source,
        field_id,
        argMax(role, (valid_from, recorded_at)) AS role
    FROM config.task_field_roles FINAL
    WHERE is_deleted = 0 AND valid_from <= now64(3)
    GROUP BY insight_source_id, data_source, field_id
)

SELECT
    insight_source_id,
    data_source,
    role,
    groupArray(field_id) AS fields_bound_to_it
FROM bound
WHERE role != 'ignored'
GROUP BY insight_source_id, data_source, role
HAVING count() > 1
LIMIT 100

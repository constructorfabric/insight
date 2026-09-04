-- The generic field snapshot replaced a CTE that coalesced over story-points
-- candidate fields per issue (specs/DATA-COMPLETENESS.md). Dropping it is only
-- safe while no issue holds a value in more than one candidate: a project's
-- style decides which field it uses, so coalescing had nothing to merge.
--
-- If this ever fails, an issue carries two story-point values and the snapshot
-- now emits both — a consumer reading by role would see two rows where it
-- expects one, and the coalescing needs to come back.

WITH candidates AS (
    SELECT DISTINCT insight_source_id, field_id
    FROM {{ ref('jira__task_field_kind') }}
    WHERE schema_custom = 'com.pyxis.greenhopper.jira:jsw-story-points'
       OR lowerUTF8(field_name) IN ('story points', 'story point estimate')
)

SELECT
    s.insight_source_id,
    s.issue_id,
    s.id_readable,
    groupArray(s.field_id) AS candidate_fields_with_values
FROM {{ ref('jira__issue_field_snapshot') }} AS s FINAL
INNER JOIN candidates AS c
    ON c.insight_source_id = s.insight_source_id
   AND c.field_id = s.field_id
WHERE length(s.value_ids) > 0
GROUP BY s.insight_source_id, s.issue_id, s.id_readable
HAVING count() > 1
LIMIT 100

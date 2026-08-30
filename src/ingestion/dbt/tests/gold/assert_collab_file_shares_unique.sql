-- One row per share scope per person, day and tool: the internal and external
-- counts unpivot into that grain, never repeated across it.
SELECT
    tenant_id,
    person_email,
    activity_date,
    tool,
    scope,
    count() AS row_count
FROM {{ ref('collab_file_shares') }}
GROUP BY tenant_id, person_email, activity_date, tool, scope
HAVING count() > 1

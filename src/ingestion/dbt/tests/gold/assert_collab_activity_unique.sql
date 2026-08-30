-- One row per person, day and tool: every tool's chat, meeting, email and
-- document facts are folded into that grain, never repeated across it.
SELECT
    tenant_id,
    person_email,
    activity_date,
    tool,
    count() AS row_count
FROM {{ ref('collab_activity') }}
GROUP BY tenant_id, person_email, activity_date, tool
HAVING count() > 1

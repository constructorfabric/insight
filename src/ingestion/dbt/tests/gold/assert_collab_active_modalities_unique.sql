-- A modality is claimed once per person, day and tool, so counting rows and
-- counting distinct modalities cannot disagree.
SELECT
    tenant_id,
    person_email,
    activity_date,
    tool,
    modality,
    count() AS row_count
FROM {{ ref('collab_active_modalities') }}
GROUP BY tenant_id, person_email, activity_date, tool, modality
HAVING count() > 1

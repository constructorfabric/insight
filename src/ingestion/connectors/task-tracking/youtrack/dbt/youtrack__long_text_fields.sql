{{ config(materialized='view', schema='staging', tags=['youtrack', 'staging']) }}

-- The fields whose values the history carries as text references: the issue
-- description, and every custom field YouTrack's catalogue types as multi-line
-- text. Decided by type, never by the length of a value.
SELECT DISTINCT tenant_id, insight_source_id, 'description' AS field_id
FROM {{ ref('youtrack__issue_observations') }}
UNION DISTINCT
SELECT DISTINCT tenant_id, insight_source_id, field_id
FROM {{ ref('youtrack__field_observations') }}
WHERE startsWith(field_type, 'text[')

-- One row per review or comment: the silver event key separates several events
-- by one person on one request, and the pull-request join must not fan it out.
SELECT
    tenant_id,
    source_id,
    event_key,
    count() AS row_count
FROM {{ ref('git_review_events') }}
GROUP BY tenant_id, source_id, event_key
HAVING count() > 1

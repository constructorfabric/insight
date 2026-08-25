-- API-to-Bronze completeness (#2419): every availability row must carry the
-- identifiers and timestamps downstream classification depends on. A NULL/empty
-- jira_id or a missing last_seen_at for a PRESENT issue means the census
-- emitted a malformed record (missing id, value, or timestamp) — the exact
-- defect class the epic's automated validation is required to detect.

SELECT
    tenant_id,
    source_id,
    jira_id,
    availability
FROM staging.jira__issue_availability_state FINAL
WHERE jira_id IS NULL
   OR jira_id = ''
   OR availability = ''
   OR (availability = 'present' AND last_seen_at = toDateTime64(0, 3))

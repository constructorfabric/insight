-- API-to-Bronze completeness (#2419): an issue the census observed as present
-- AND that the incremental scan enumerated (a jira_issue_keys row exists) must
-- have its full bronze_jira.jira_issue record. A violation means the sync
-- committed the lightweight parent row but lost the full-record emission —
-- exactly the green-but-empty failure mode this check exists to catch.
--
-- The census alone is not enough to demand a full record: issues untouched
-- since jira_start_date are censused but legitimately never scanned in full.

SELECT
    av.tenant_id,
    av.source_id,
    av.jira_id
FROM staging.jira__issue_availability_state AS av FINAL
INNER JOIN bronze_jira.jira_issue_keys AS ik FINAL
    ON ik.tenant_id = av.tenant_id
    AND ik.source_id = av.source_id
    AND ik.jira_id = av.jira_id
LEFT ANTI JOIN bronze_jira.jira_issue AS i FINAL
    ON i.tenant_id = av.tenant_id
    AND i.source_id = av.source_id
    AND i.jira_id = av.jira_id
WHERE av.availability = 'present'

-- API-to-Bronze completeness (#2419): an issue the census observed as present
-- AND that the incremental scan enumerated (a jira_issue_keys row exists) must
-- have its full bronze_jira.jira_issue record. A violation means the sync
-- committed the lightweight parent row but lost the full-record emission —
-- exactly the green-but-empty failure mode this check exists to catch.
--
-- The census alone is not enough to demand a full record: issues untouched
-- since jira_start_date are censused but legitimately never scanned in full.
--
-- The two streams scan a project at slightly different moments inside one
-- sync, so an issue updated mid-sync can land in jira_issue_keys before
-- jira_issue has seen it — the next incremental sync closes that gap on its
-- own. Only flag issues whose key-row `updated` predates the full stream's
-- last scan by more than the race window.

WITH issue_scan AS (
    -- Per (tenant, source): each source instance scans on its own clock, so a
    -- global watermark would judge one instance's issues against another's
    -- scan time and report rows inside their own race window.
    SELECT
        tenant_id,
        source_id,
        max(_airbyte_extracted_at) AS scanned_at
    FROM bronze_jira.jira_issue
    GROUP BY tenant_id, source_id
)

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
INNER JOIN issue_scan AS scan
    ON scan.tenant_id = av.tenant_id
    AND scan.source_id = av.source_id
WHERE av.availability = 'present'
  AND parseDateTime64BestEffortOrNull(ik.updated, 3) < scan.scanned_at - INTERVAL 1 HOUR

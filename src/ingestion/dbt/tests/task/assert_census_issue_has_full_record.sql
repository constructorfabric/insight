{{ config(
    tags=['connector_quality', 'jira'],
    store_failures=true,
    meta={
        'title': 'A censused issue of the recent window has its full record',
        'domain': 'task-tracking',
        'category': 'completeness',
        'tier': 'error',
        'remediation': 'The census enumerated an issue the incremental scan never fetched, so bronze holds no record for it and every substream keyed on the issue -- changelog, comments, worklogs -- is empty too. Nothing downstream can see the omission, because every relation simply lacks the issue. Check the slice bounds the issue stream sends: an issue whose `updated` is always newer than the slice ceiling is never inside any slice, and the lookback on the floor does not reach it. Compare the newest `updated` bronze holds against the time the scan ran; a gap that never closes is the ceiling, not the source.'
    }
) }}

-- API-to-Bronze completeness (#2419): an issue the census observed as present,
-- and that is recent enough to be inside the incremental stream's remit, must
-- have its full bronze_jira.jira_issue record. A violation means the sync never
-- fetched the issue at all -- the green-but-empty failure mode this check
-- exists to catch.
--
-- The census alone is not enough to demand a full record: issues untouched
-- since jira_start_date are censused but legitimately never scanned in full.
-- What bounds the demand here is the issue id rather than a date. Jira assigns
-- ids in creation order, so the smallest id among issues created inside the
-- recent window is a floor: every id at or above it belongs to an issue created
-- inside that window, hence after any plausible start date, hence one the
-- incremental scan is required to hold.
--
-- The floor is derived rather than configured for two reasons. The start date
-- is connector config and is not in the warehouse; and the census row cannot
-- carry a creation date, because the census asks for `id` alone -- which is
-- what lets Jira serve it in 5000-row pages -- and a second field would cost
-- that page size across every issue in the instance.
--
-- Deliberately NOT gated on a jira_issue_keys row. The lightweight stream
-- filters and sorts on the same mutating cursor as the full one, so the two
-- fail together: requiring a key row makes the check blind to exactly the case
-- where nothing about the issue was fetched, which is the severe one.
--
-- The race window is measured from FIRST sighting, not last. The census scans a
-- project after the issue stream inside one sync, so an issue's latest sighting
-- is always younger than the scan and a guard written against it would never
-- fire. An issue first censused more than the race window before the scan has
-- had at least one whole sync in which to be fetched.

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
),

recent_floor AS (
    SELECT
        tenant_id,
        source_id,
        min(toUInt64OrZero(jira_id)) AS floor_id
    FROM bronze_jira.jira_issue FINAL
    WHERE parseDateTime64BestEffortOrNull(created, 3) >= now() - INTERVAL 30 DAY
    GROUP BY tenant_id, source_id
),

first_seen AS (
    SELECT
        tenant_id,
        source_id,
        entity_id AS jira_id,
        min(updated_at) AS censused_at
    FROM staging.jira__issue_availability_history
    WHERE field_name = 'availability'
    GROUP BY tenant_id, source_id, entity_id
)

SELECT
    av.tenant_id,
    av.source_id,
    av.jira_id
FROM staging.jira__issue_availability_state AS av FINAL
INNER JOIN recent_floor AS f
    ON f.tenant_id = av.tenant_id
    AND f.source_id = av.source_id
INNER JOIN issue_scan AS scan
    ON scan.tenant_id = av.tenant_id
    AND scan.source_id = av.source_id
INNER JOIN first_seen AS fs
    ON fs.tenant_id = av.tenant_id
    AND fs.source_id = av.source_id
    AND fs.jira_id = av.jira_id
LEFT ANTI JOIN bronze_jira.jira_issue AS i FINAL
    ON i.tenant_id = av.tenant_id
    AND i.source_id = av.source_id
    AND i.jira_id = av.jira_id
WHERE av.availability = 'present'
  AND toUInt64OrZero(av.jira_id) >= f.floor_id
  AND fs.censused_at < scan.scanned_at - INTERVAL 1 HOUR

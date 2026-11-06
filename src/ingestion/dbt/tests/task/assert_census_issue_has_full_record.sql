{{ config(
    tags=['connector_quality', 'jira'],
    store_failures=true,
    meta={
        'title': 'A censused issue that changed before the last scan has its full record',
        'domain': 'task-tracking',
        'category': 'completeness',
        'tier': 'error',
        'remediation': 'The census sees the issue and the warehouse holds evidence it changed before the last issue scan ran -- the scan enumerated it into jira_issue_keys, a worklog on it changed, or it was created inside the recent window -- yet bronze holds no jira_issue record for it, so every substream keyed on the issue is empty too and nothing downstream can see the omission. Compare the issue scan slices against the change time the evidence carries: a change the slices never cover is a cursor or bound defect, not the source.'
    }
) }}

-- Full contract and rationale: connectors/task-tracking/jira/specs/DATA-COMPLETENESS.md.
-- INVARIANT: every evidence arm is bounded by jira_start_date the way the issue
-- scan is, so a change it reports is one the scan was required to collect.

WITH issue_scan AS (
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
),

censused AS (
    SELECT
        tenant_id,
        source_id,
        jira_id
    FROM bronze_jira.jira_issue_census FINAL
),

created_in_window AS (
    SELECT
        c.tenant_id AS tenant_id,
        c.source_id AS source_id,
        c.jira_id AS jira_id,
        fs.censused_at AS changed_at
    FROM censused AS c
    INNER JOIN recent_floor AS f
        ON f.tenant_id = c.tenant_id
        AND f.source_id = c.source_id
    INNER JOIN first_seen AS fs
        ON fs.tenant_id = c.tenant_id
        AND fs.source_id = c.source_id
        AND fs.jira_id = c.jira_id
    WHERE toUInt64OrZero(c.jira_id) >= f.floor_id
),

enumerated_by_scan AS (
    SELECT
        tenant_id,
        source_id,
        jira_id,
        parseDateTime64BestEffortOrNull(updated, 3) AS changed_at
    FROM bronze_jira.jira_issue_keys FINAL
),

worklog_changes AS (
    SELECT
        tenant_id,
        source_id,
        jira_id,
        min(parseDateTime64BestEffortOrNull(updated, 3)) AS changed_at
    FROM bronze_jira.jira_worklogs FINAL
    GROUP BY tenant_id, source_id, jira_id
),

change_evidence AS (
    SELECT tenant_id, source_id, jira_id, changed_at FROM created_in_window
    UNION ALL
    SELECT tenant_id, source_id, jira_id, changed_at FROM enumerated_by_scan
    UNION ALL
    SELECT tenant_id, source_id, jira_id, changed_at FROM worklog_changes
),

changed_before_scan AS (
    SELECT DISTINCT
        e.tenant_id AS tenant_id,
        e.source_id AS source_id,
        e.jira_id AS jira_id
    FROM change_evidence AS e
    INNER JOIN issue_scan AS scan
        ON scan.tenant_id = e.tenant_id
        AND scan.source_id = e.source_id
    WHERE e.changed_at < scan.scanned_at - INTERVAL 1 HOUR
)

SELECT
    av.tenant_id AS tenant_id,
    av.source_id AS source_id,
    av.jira_id AS jira_id
FROM staging.jira__issue_availability_state AS av FINAL
INNER JOIN censused AS c
    ON c.tenant_id = av.tenant_id
    AND c.source_id = av.source_id
    AND c.jira_id = av.jira_id
INNER JOIN changed_before_scan AS ch
    ON ch.tenant_id = av.tenant_id
    AND ch.source_id = av.source_id
    AND ch.jira_id = av.jira_id
LEFT ANTI JOIN bronze_jira.jira_issue AS i FINAL
    ON i.tenant_id = av.tenant_id
    AND i.source_id = av.source_id
    AND i.jira_id = av.jira_id
WHERE av.availability = 'present'

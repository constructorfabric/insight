{{ config(
    severity='warn',
    tags=['connector_quality', 'jira'],
    store_failures=true,
    meta={
        'title': 'Jira changelog, comment and worklog rows name their issue',
        'domain': 'task-tracking',
        'category': 'completeness',
        'tier': 'warn',
        'remediation': 'A changelog entry, comment or worklog whose row carries no issue id cannot be attributed to an issue and is left out of the field-history journal and the lifecycle events. Connector 6.1.0 stamps the id on every new row and the deploy heal fills older rows from the issue streams, so a remaining row names an issue those streams never delivered — deleted at the source before it was fetched, or outside the configured window. Confirm the issue is absent from jira_issue and jira_issue_keys; if it is present, the heal did not run on this table.'
    }
) }}

-- Rows the journal deliberately leaves out, counted so the omission is visible.
--
-- The issue-scoped substreams are partitioned by the issue KEY, which Jira
-- changes on a move between projects, so the key alone cannot attribute a row.
-- Every row is expected to carry `jira_id`; one that does not is reported here
-- per table, with the keys involved, rather than silently dropped.

SELECT 'jira_issue_history' AS bronze_table, source_id, count() AS rows_without_issue_id, groupUniqArray(10)(id_readable) AS sample_keys
FROM {{ source('bronze_jira', 'jira_issue_history') }} FINAL
WHERE jira_id IS NULL
GROUP BY source_id

UNION ALL

SELECT 'jira_comments' AS bronze_table, source_id, count() AS rows_without_issue_id, groupUniqArray(10)(id_readable) AS sample_keys
FROM {{ source('bronze_jira', 'jira_comments') }} FINAL
WHERE jira_id IS NULL
GROUP BY source_id

UNION ALL

SELECT 'jira_worklogs' AS bronze_table, source_id, count() AS rows_without_issue_id, groupUniqArray(10)(id_readable) AS sample_keys
FROM {{ source('bronze_jira', 'jira_worklogs') }} FINAL
WHERE jira_id IS NULL
GROUP BY source_id

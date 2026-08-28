-- Regression guard for the `items` double-encoding defect: a changelog row
-- whose `items` is an escaped JSON string instead of an array reads as a scalar
-- through JSONExtractArrayRaw, so jira__changelog_items skips it and the field
-- history silently loses every affected event. The stream declares `items` as
-- a plain string to prevent this; the failure was invisible because the
-- pipeline stays green while the journal quietly shrinks.

SELECT
    unique_key,
    id_readable,
    created,
    substring(items, 1, 80) AS items_head
FROM {{ source('bronze_jira', 'jira_issue_history') }}
WHERE startsWith(items, '"')

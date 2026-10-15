-- Unit test for `jira_changelog_item_field_id`: which field a raw changelog item
-- changes, including the system fields Jira names by label with no `fieldId`.
--
-- Fixtures are literal and synthetic — this test touches no table.

WITH fixtures AS (
    SELECT
        t.1 AS label,
        t.2 AS item,
        t.3 AS expected
    FROM (
        SELECT arrayJoin([
            ('field id present',
             '{"field": "status", "fieldId": "status", "fieldtype": "jira", "from": "1", "to": "3"}',
             'status'),
            ('link written with its field id',
             '{"field": "issuelinks", "fieldId": "issuelinks", "fieldtype": "jira", "to": "TST-9", "toString": "This issue relates to TST-9"}',
             'issuelinks'),
            ('link written by label only',
             '{"field": "Link", "fieldtype": "jira", "to": "TST-9", "toString": "This issue relates to TST-9"}',
             'issuelinks'),
            ('parent changed',
             '{"field": "IssueParentAssociation", "fieldtype": "jira", "to": "10001", "toString": "TST-1"}',
             'parent'),
            ('parent changed, older label',
             '{"field": "Parent", "fieldtype": "jira", "to": "10001", "toString": "TST-1"}',
             'parent'),
            ('custom field named like a system one',
             '{"field": "Link", "fieldtype": "custom", "to": "x"}',
             ''),
            ('worklog event',
             '{"field": "WorklogId", "fieldtype": "jira", "to": "70001"}',
             ''),
            ('remote link event',
             '{"field": "RemoteIssueLink", "fieldtype": "jira", "to": "70002"}',
             ''),
            ('issue moved',
             '{"field": "Key", "fieldtype": "jira", "from": "OLD-1", "to": "NEW-1"}',
             '')
        ]) AS t
    )
)

SELECT
    label,
    expected,
    {{ jira_changelog_item_field_id('item') }} AS actual
FROM fixtures
WHERE actual != expected

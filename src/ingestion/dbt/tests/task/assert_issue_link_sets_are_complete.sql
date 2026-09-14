-- A link set that was cut off at its page boundary.
--
-- `issue_links` reads five nested GraphQL connections, and a declarative
-- manifest cannot paginate a nested connection: it asks for a page and takes
-- what it gets. An issue with more sub-issues, dependencies or closing pull
-- requests than the page holds therefore loses its tail — and loses it
-- SILENTLY, which is the part that matters. `github__task_links` would then
-- report fewer links than exist and give no sign that anything is missing.
--
-- The vendor states the true size of each set, so the two are compared here.
-- Firing means the page has to grow, or the stream has to move to a component
-- that can follow a nested cursor.

WITH sets AS (
    SELECT
        COALESCE(source_id, '')                                 AS insight_source_id,
        concat(COALESCE(repo_full_name, ''), '#', toString(COALESCE(item_number, 0))) AS id_readable,
        arrayJoin([
            ('sub_issues',   JSONLength(COALESCE(sub_issues_json, '[]')),             COALESCE(sub_issues_total, 0)),
            ('blocked_by',   JSONLength(COALESCE(blocked_by_json, '[]')),             COALESCE(blocked_by_total, 0)),
            ('blocking',     JSONLength(COALESCE(blocking_json, '[]')),               COALESCE(blocking_total, 0)),
            ('closed_by_pr', JSONLength(COALESCE(closed_by_pull_requests_json, '[]')), COALESCE(closed_by_pull_requests_total, 0))
        ]) AS s
    FROM {{ source('bronze_github', 'issue_links') }} FINAL
)

SELECT
    insight_source_id,
    id_readable,
    s.1 AS link_set,
    s.2 AS collected,
    s.3 AS stated_by_vendor
FROM sets
WHERE s.2 < s.3
LIMIT 100

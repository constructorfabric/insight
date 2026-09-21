-- The board card stream sweeps every board and keeps what it gets. A sweep that
-- comes back empty is indistinguishable from a genuinely quiet week: GraphQL
-- answers 200 whether the board is empty, the token lost its project scope, or
-- a page was dropped. This guards the outcome rather than any one cause.
--
-- It used to guard one cause in particular — a server-side
-- `items(query: "updated:>=…")` filter that returned zero rows with no error
-- when its qualifier was unparseable. That filter is gone, and the stream no
-- longer narrows itself at all, so the silent-filter reading of a zero no
-- longer applies. The check survives the filter it was written for because
-- what it actually asserts is completeness, not the filter's syntax.
--
-- The condition is deliberately narrow so it cannot fire on a real zero: the
-- source must have collected board field definitions (boards are visible) AND
-- board status events (a card existed and moved) while holding no card rows at
-- all. A new organization with empty boards satisfies neither of the first two.

WITH boards AS (
    SELECT DISTINCT tenant_id, source_id
    FROM {{ source('bronze_github', 'project_fields') }} FINAL
    WHERE COALESCE(project_id, '') != ''
),

board_events AS (
    SELECT DISTINCT tenant_id, source_id
    FROM {{ source('bronze_github', 'issue_timeline_events') }} FINAL
    WHERE event_type = 'ProjectV2ItemStatusChangedEvent'
),

cards AS (
    SELECT DISTINCT tenant_id, source_id
    FROM {{ source('bronze_github', 'project_items') }} FINAL
    WHERE COALESCE(item_id, '') != ''
)

SELECT
    b.tenant_id  AS tenant_id,
    b.source_id  AS source_id,
    'boards and board events are collected, cards are not — the sweep returned nothing' AS finding
FROM boards AS b
INNER JOIN board_events AS e
    ON e.tenant_id = b.tenant_id AND e.source_id = b.source_id
LEFT ANTI JOIN cards AS c
    ON c.tenant_id = b.tenant_id AND c.source_id = b.source_id
LIMIT 100

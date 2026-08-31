-- The board card stream filters server-side on a date: `items(query: "updated:>=…")`.
-- That filter fails SILENTLY. An unparseable qualifier — a stray timestamp, a
-- typo — returns zero rows with no GraphQL error, so the sync is green, the
-- stream is empty, and nothing distinguishes it from a genuinely quiet week.
-- The connector guards the input by rendering a bare date; this guards the
-- outcome.
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
    'boards and board events are collected, cards are not — the incremental filter is rejected' AS finding
FROM boards AS b
INNER JOIN board_events AS e
    ON e.tenant_id = b.tenant_id AND e.source_id = b.source_id
LEFT ANTI JOIN cards AS c
    ON c.tenant_id = b.tenant_id AND c.source_id = b.source_id
LIMIT 100

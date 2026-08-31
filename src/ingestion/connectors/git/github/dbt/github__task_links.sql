-- depends_on: {{ ref('github__bronze_promoted') }}
{{ config(
    materialized='table',
    alias='github__task_links',
    schema='staging',
    tags=['github', 'staging', 'silver:class_task_links']
) }}

-- One row per link OCCURRENCE, with the interval it held: `valid_from` when it
-- appeared, `valid_to` when it went away, NULL while it is still there. The
-- question this answers is "which links existed between two dates", which is a
-- range predicate and nothing more:
--
--   WHERE valid_from < :to AND (valid_to IS NULL OR valid_to > :from)
--
-- INVARIANT: `valid_from` belongs to the KEY, not to a version. A link removed
-- and re-added later is two occurrences and two rows, and both are current
-- facts about different intervals — unlike a snapshot version, which would be
-- the same fact restated (ADR-0001, ADR-0004).
--
-- `evidence` says how the interval was established, because the two sources
-- differ in precision and a consumer must not average them:
--   `event`       — an add/remove pair from the issue timeline. Exact to the
--                   second, and the vendor's own record of the change.
--   `observation` — the link was seen present in successive snapshots. The
--                   bounds are first-seen and last-seen, so the true edges lie
--                   somewhere between two syncs.
--
-- Which source rules which link type is not a preference. Hierarchy and
-- dependency links emit a matching add/remove pair, so they fold exactly. A
-- pull request closing an issue does not: the timeline may report it as a
-- connection, as a cross-reference that claims it will NOT close the issue, or
-- not at all, while `closedByPullRequestsReferences` states it plainly. So
-- that one kind is observed and the rest are folded, and no link type is ever
-- built from both.

WITH events AS (
    SELECT
        insight_source_id, tenant_id, id_readable, link_type, link_action,
        target_type, target_readable, is_cross_repository, actor_display, actor_id,
        event_at, event_id, collected_at
    FROM {{ ref('github__issue_link_events') }}
),

adds AS (
    SELECT * FROM events WHERE link_action = 'add'
),

removes AS (
    SELECT
        insight_source_id, id_readable, link_type, target_readable,
        event_at                                                AS removed_at,
        actor_display                                           AS removed_by,
        collected_at                                            AS removed_collected_at,
        -- INVARIANT: an ASOF LEFT JOIN that matches nothing yields the column
        -- TYPE'S DEFAULT, not NULL — an unclosed interval would read as closed
        -- at the epoch, which is a date, and every range query would believe
        -- it. This flag is how the join says "no match" out loud.
        toUInt8(1)                                              AS matched
    FROM events WHERE link_action = 'remove'
),

-- ASOF pairs every add with the nearest remove that follows it in the same
-- link. A strict `<` is deliberate: an add and a remove sharing one timestamp
-- are not a closed interval, they are a collection artefact, and pairing them
-- would report a link that existed for zero seconds.
event_intervals AS (
    SELECT
        a.insight_source_id                                     AS insight_source_id,
        a.tenant_id                                             AS tenant_id,
        a.id_readable                                           AS id_readable,
        a.link_type                                             AS link_type,
        a.target_type                                           AS target_type,
        a.target_readable                                       AS target_readable,
        a.is_cross_repository                                   AS is_cross_repository,
        a.event_at                                              AS valid_from,
        toUInt8(1)                                              AS valid_from_known,
        if(r.matched = 1, toNullable(r.removed_at), NULL)       AS valid_to,
        a.actor_display                                         AS added_by,
        if(r.matched = 1, r.removed_by, CAST(NULL AS Nullable(String))) AS removed_by,
        'event'                                                 AS evidence,
        a.event_id                                              AS origin_event_id,
        -- INVARIANT: the newest evidence behind the row, not the addition's.
        -- `_version` derives from this and `class_task_links` is incremental on
        -- it, so a row whose only change is that it CLOSED would be filtered
        -- out and the interval would stay open in silver forever.
        if(r.matched = 1, greatest(a.collected_at, r.removed_collected_at), a.collected_at) AS collected_at
    FROM adds AS a
    ASOF LEFT JOIN removes AS r
        ON a.insight_source_id = r.insight_source_id
        AND a.id_readable = r.id_readable
        AND a.link_type = r.link_type
        AND a.target_readable = r.target_readable
        AND a.event_at < r.removed_at
),

-- A removal whose addition predates everything collected. Dropping it would
-- report the link as never having existed; inventing a start would report a
-- date nobody observed. The interval is left open at the bottom and flagged,
-- so a range query still finds it and a reader still knows the edge is a
-- bound rather than a fact.
orphan_removes AS (
    SELECT
        insight_source_id                                       AS insight_source_id,
        tenant_id                                               AS tenant_id,
        id_readable                                             AS id_readable,
        link_type                                               AS link_type,
        target_type                                             AS target_type,
        target_readable                                         AS target_readable,
        is_cross_repository                                     AS is_cross_repository,
        toDateTime64(0, 3)                                      AS valid_from,
        toUInt8(0)                                              AS valid_from_known,
        first_at                                                AS valid_to,
        CAST(NULL AS Nullable(String))                          AS added_by,
        first_actor                                             AS removed_by,
        'event'                                                 AS evidence,
        ''                                                      AS origin_event_id,
        collected_at                                            AS collected_at
    FROM (
        SELECT
            insight_source_id,
            id_readable,
            link_type,
            target_readable,
            any(tenant_id)                                             AS tenant_id,
            argMin(link_action, (event_at, event_id))                  AS first_action,
            min(event_at)                                              AS first_at,
            argMin(target_type, (event_at, event_id))                  AS target_type,
            argMin(is_cross_repository, (event_at, event_id))          AS is_cross_repository,
            argMin(actor_display, (event_at, event_id))                AS first_actor,
            max(collected_at)                                          AS collected_at
        FROM events
        GROUP BY insight_source_id, id_readable, link_type, target_readable
    )
    -- The earliest event of a link being a removal means its addition happened
    -- before anything collected. Any later add/remove pair in the same link is
    -- already handled by the ASOF pairing above.
    WHERE first_action = 'remove'
),

-- Observation side: one row per (issue, closing pull request) per snapshot
-- version, so the version timestamps bound the link.
pr_observations AS (
    SELECT
        COALESCE(s.source_id, '')                               AS insight_source_id,
        COALESCE(s.tenant_id, '')                               AS tenant_id,
        concat(COALESCE(s.repo_full_name, ''), '#', toString(COALESCE(s.item_number, 0))) AS id_readable,
        concat(
            JSONExtractString(JSONExtractRaw(link_raw, 'repository'), 'nameWithOwner'),
            '#', toString(JSONExtractInt(link_raw, 'number'))
        )                                                       AS target_readable,
        COALESCE(s.repo_full_name, '')                          AS source_repo,
        s._tracked_at                                           AS observed_at
    FROM (
        SELECT *, arrayJoin(JSONExtractArrayRaw(COALESCE(closed_by_pull_requests_json, '[]'))) AS link_raw
        FROM {{ ref('github__issue_links_snapshot') }}
    ) AS s
),

-- INVARIANT: the newest version of an issue's links, taken from the snapshot
-- itself and NOT from `pr_observations`. A version whose link set is empty
-- produces no observation row, so deriving the latest version from the
-- observations would take the last version that still HAD a link — and every
-- link removed by emptying the set would read as still open.
latest_observation AS (
    SELECT
        COALESCE(source_id, '')                                 AS insight_source_id,
        concat(COALESCE(repo_full_name, ''), '#', toString(COALESCE(item_number, 0))) AS id_readable,
        max(_tracked_at)                                        AS last_version_at
    FROM {{ ref('github__issue_links_snapshot') }}
    GROUP BY insight_source_id, id_readable
),

pr_intervals AS (
    SELECT
        o.insight_source_id                                     AS insight_source_id,
        any(o.tenant_id)                                        AS tenant_id,
        o.id_readable                                           AS id_readable,
        'closed_by_pr'                                          AS link_type,
        'pull_request'                                          AS target_type,
        o.target_readable                                       AS target_readable,
        -- The closing pull request may live in another repository, so this is
        -- a comparison and not a constant.
        any(o.source_repo) != splitByChar('#', o.target_readable)[1] AS is_cross_repository,
        min(o.observed_at)                                      AS valid_from,
        -- First SEEN, not first true: the link may predate the first snapshot
        -- that carried it, so the lower edge is a bound like an orphan's.
        toUInt8(0)                                              AS valid_from_known,
        -- Still in the newest version of this issue's links -> still there.
        if(max(o.observed_at) = any(l.last_version_at), CAST(NULL AS Nullable(DateTime64(3))), max(o.observed_at)) AS valid_to,
        CAST(NULL AS Nullable(String))                          AS added_by,
        CAST(NULL AS Nullable(String))                          AS removed_by,
        'observation'                                           AS evidence,
        ''                                                      AS origin_event_id,
        -- The issue's newest version, for the same reason: a link closes
        -- because a LATER observation no longer carries it, and that evidence
        -- has to move `_version` or silver never sees the closure.
        any(l.last_version_at)                                  AS collected_at
    FROM pr_observations AS o
    INNER JOIN latest_observation AS l
        ON l.insight_source_id = o.insight_source_id AND l.id_readable = o.id_readable
    GROUP BY o.insight_source_id, o.id_readable, o.target_readable
),

every_interval AS (
    SELECT * FROM event_intervals
    UNION ALL
    SELECT * FROM orphan_removes
    UNION ALL
    SELECT * FROM pr_intervals
)

SELECT
    CAST(concat(
        insight_source_id, '-github-', id_readable, '-', link_type, '-',
        target_readable, '-', toString(toUnixTimestamp64Milli(valid_from))
    ) AS String)                                                AS unique_key,
    CAST(insight_source_id AS String)                           AS insight_source_id,
    CAST('github' AS String)                                    AS data_source,
    CAST(id_readable AS String)                                 AS id_readable,
    CAST(link_type AS String)                                   AS link_type,
    CAST(target_type AS Enum8('issue' = 1, 'pull_request' = 2)) AS target_type,
    CAST(target_readable AS String)                             AS target_readable,
    CAST(is_cross_repository AS Bool)                           AS is_cross_repository,
    valid_from                                                  AS valid_from,
    valid_from_known                                            AS valid_from_known,
    valid_to                                                    AS valid_to,
    CAST(added_by AS Nullable(String))                          AS added_by,
    CAST(removed_by AS Nullable(String))                        AS removed_by,
    CAST(evidence AS Enum8('event' = 1, 'observation' = 2))     AS evidence,
    CAST(nullIf(origin_event_id, '') AS Nullable(String))       AS origin_event_id,
    toDateTime64(collected_at, 3)                               AS collected_at,
    CAST(toUnixTimestamp64Milli(toDateTime64(collected_at, 3)) AS UInt64) AS _version
FROM every_interval

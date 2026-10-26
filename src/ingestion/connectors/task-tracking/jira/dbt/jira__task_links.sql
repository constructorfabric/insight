{{ config(
    materialized='table',
    alias='jira__task_links',
    schema='staging',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    query_settings={'join_use_nulls': 1},
    tags=['jira', 'staging', 'silver:class_task_links']
) }}

-- One row per link OCCURRENCE, folded from the changelog's own add and remove
-- events (`evidence = event`). See `class_task_links` for the contract.
--
-- The issue's newest read has the last word. A link can go without an event —
-- Jira logs nothing on the other issue when one is deleted, nor on either when
-- a bulk change drops it — and come back without one. Where the read disagrees
-- with the state the events leave, the occurrence closes or reopens at an
-- observed edge (`evidence = observation`): the issue's `updated` when that is
-- after the last event, else the read itself.
--
-- An occurrence is one link of one type from one issue to one linked issue, as
-- `jira__link_occurrences` identifies it. Jira logs a link on both issues, each
-- in its own words, so each side is read from its own changelog and nothing is
-- mirrored. A parent change is logged on the child alone, so its `sub_issue`
-- side is mirrored onto the parent.
--
-- A link the issue held before its first event on that occurrence has no add:
-- it is the first event's removal, or the snapshot's link when no event touched
-- it. Its interval opens at the issue's creation, the earliest it can have
-- existed, with `valid_from_known = 0`.
--
-- A linked issue whose key resolved to no id leaves `target_id` NULL.

WITH held_now AS (
    SELECT insight_source_id, kind, type_id, type_name, target_id, target_key, extracted_at
    FROM {{ ref('jira__issue_link_snapshot') }} FINAL
),

-- The issue's current key, its creation, and its newest read, from its newest
-- bronze row.
issues AS (
    SELECT
        COALESCE(source_id, '')                                       AS insight_source_id,
        COALESCE(toString(jira_id), '')                               AS issue_id,
        argMax(COALESCE(toString(id_readable), ''), _airbyte_extracted_at) AS current_key,
        argMax(parseDateTime64BestEffortOrNull(created, 3), _airbyte_extracted_at) AS created_at,
        argMax(parseDateTime64BestEffortOrNull(updated, 3), _airbyte_extracted_at) AS updated_at,
        toDateTime64(max(_airbyte_extracted_at), 3)                   AS read_at
    FROM {{ source('bronze_jira', 'jira_issue') }}
    WHERE jira_id IS NOT NULL
    GROUP BY insight_source_id, issue_id
),

-- The current key of an issue the connector does not sync, as the newest
-- snapshot linking to it names it.
linked_keys AS (
    SELECT
        insight_source_id,
        target_id                                                     AS issue_id,
        argMax(target_key, extracted_at)                              AS current_key
    FROM held_now
    WHERE target_id != '' AND target_key != ''
    GROUP BY insight_source_id, issue_id
),

-- A link type's current name, by its immutable id.
link_types AS (
    SELECT
        insight_source_id,
        type_id,
        argMax(type_name, extracted_at)                               AS type_name
    FROM held_now
    WHERE kind = 'issuelink' AND type_id != ''
    GROUP BY insight_source_id, type_id
),

occurrences AS (
    SELECT
        o.insight_source_id AS insight_source_id, o.issue_id AS issue_id, o.kind AS kind,
        o.occurrence_key AS occurrence_key, o.type_id AS type_id, o.direction AS direction, o.phrase AS phrase,
        o.target_ident AS target_ident, o.issue_key AS issue_key, o.target_id AS target_id,
        o.target_key AS target_key, o.ops AS ops, o.held_extracted_at AS held_extracted_at,
        o.held_before AS held_before, r.updated_at AS updated_at, r.read_at AS read_at
    FROM {{ ref('jira__link_occurrences') }} AS o FINAL
    LEFT JOIN issues AS r
        ON r.insight_source_id = o.insight_source_id
       AND r.issue_id = o.issue_id
),

-- Where the newest read disagrees with the events. A read older than the last
-- event predates it and says nothing.
observed AS (
    SELECT
        *,
        notEmpty(ops)
            AND read_at IS NOT NULL
            AND ops[-1].1 <= read_at
            AND (held_extracted_at IS NOT NULL) != (ops[-1].3 = 'add')    AS read_disagrees,
        multiIf(NOT read_disagrees,               '',
                held_extracted_at IS NOT NULL,    'add',
                                                  'remove')               AS observed_action,
        if(COALESCE(updated_at, toDateTime64(0, 3)) > ops[-1].1,
           least(assumeNotNull(updated_at), assumeNotNull(read_at)),
           read_at)                                                       AS observed_at,
        toUInt32(length(ops) + 1)                                         AS observed_position
    FROM occurrences
),

-- Positions in `ops` where the occurrence appears and disappears; 0 is "held
-- before the first operation", `observed_position` the newest read. They
-- alternate by construction, so the n-th opening pairs with the n-th closing.
transitions AS (
    SELECT
        *,
        arrayMap(k -> if(k = 1, held_before, ops[k - 1].3 = 'add'), arrayEnumerate(ops)) AS present_before,
        arrayConcat(if(held_before, [toUInt32(0)], []),
                    arrayFilter(k -> ops[k].3 = 'add' AND NOT present_before[k], arrayEnumerate(ops)),
                    if(observed_action = 'add', [observed_position], [])) AS openings,
        arrayConcat(arrayFilter(k -> ops[k].3 = 'remove' AND present_before[k], arrayEnumerate(ops)),
                    if(observed_action = 'remove', [observed_position], [])) AS closings
    FROM observed
),

intervals AS (
    SELECT
        t.*,
        arrayJoin(arrayEnumerate(openings))                           AS n,
        openings[n]                                                   AS opened_by,
        if(n <= length(closings), closings[n], 0)                     AS closed_by
    FROM transitions AS t
),

owned AS (
    SELECT
        i.insight_source_id                                           AS insight_source_id,
        i.issue_id                                                    AS issue_id,
        COALESCE(nullIf(COALESCE(owner.current_key, ''), ''), i.issue_key) AS id_readable,
        if(i.kind = 'parent', 'parent',
           {{ jira_link_type("COALESCE(lt.type_name, '')", 'i.direction', 'i.phrase') }}) AS link_type,
        i.occurrence_key                                              AS occurrence_key,
        i.target_ident                                                AS target_ident,
        nullIf(i.target_id, '')                                       AS target_id,
        COALESCE(nullIf(COALESCE(target.current_key, ''), ''),
                 nullIf(COALESCE(linked.current_key, ''), ''),
                 i.target_key)                                        AS target_readable,
        -- No creation date means no issue row to read it from; the epoch then
        -- says "unknown", as `valid_from_known = 0` lets it. A link the read
        -- brought back reappeared after its last removal, the bound it opens at.
        multiIf(i.opened_by = 0,                   COALESCE(owner.created_at, toDateTime64(0, 3)),
                i.opened_by = i.observed_position, i.ops[-1].1,
                                                   i.ops[i.opened_by].1) AS valid_from,
        toUInt8(i.opened_by != 0 AND i.opened_by != i.observed_position) AS valid_from_known,
        multiIf(i.closed_by = 0,                   CAST(NULL AS Nullable(DateTime64(3))),
                i.closed_by = i.observed_position, toNullable(toDateTime64(i.observed_at, 3)),
                                                   toNullable(i.ops[i.closed_by].1)) AS valid_to,
        if(i.opened_by = 0 OR i.opened_by = i.observed_position, CAST(NULL AS Nullable(String)), i.ops[i.opened_by].4) AS added_by,
        if(i.closed_by = 0 OR i.closed_by = i.observed_position, CAST(NULL AS Nullable(String)), i.ops[i.closed_by].4) AS removed_by,
        if(i.opened_by = 0 OR i.opened_by = i.observed_position, CAST(NULL AS Nullable(String)),
           toNullable(i.ops[i.opened_by].5))                          AS origin_event_id,
        if(i.opened_by = i.observed_position OR i.closed_by = i.observed_position, 'observation', 'event') AS evidence,
        -- INVARIANT: the newest evidence behind the row, so an interval that
        -- only CLOSED still moves `_version`.
        greatest(
            multiIf(i.opened_by = 0,                   COALESCE(i.held_extracted_at, toDateTime64(0, 3)),
                    i.opened_by = i.observed_position, COALESCE(i.read_at, toDateTime64(0, 3)),
                                                       i.ops[i.opened_by].6),
            multiIf(i.closed_by = 0,                   toDateTime64(0, 3),
                    i.closed_by = i.observed_position, COALESCE(i.read_at, toDateTime64(0, 3)),
                                                       i.ops[i.closed_by].6)
        )                                                             AS collected_at
    FROM intervals AS i
    LEFT JOIN issues AS owner
        ON owner.insight_source_id = i.insight_source_id
       AND owner.issue_id = i.issue_id
    LEFT JOIN issues AS target
        ON target.insight_source_id = i.insight_source_id
       AND target.issue_id = i.target_id
    LEFT JOIN linked_keys AS linked
        ON linked.insight_source_id = i.insight_source_id
       AND linked.issue_id = i.target_id
    LEFT JOIN link_types AS lt
        ON lt.insight_source_id = i.insight_source_id
       AND lt.type_id = i.type_id
),

-- A link that appeared and went in one instant never held.
lasting AS (
    SELECT * FROM owned
    WHERE valid_to IS NULL OR valid_to > valid_from
),

-- The parent's side of each parent interval. A parent known only by key is
-- no issue a row can belong to.
mirrored AS (
    SELECT
        l.insight_source_id                                           AS insight_source_id,
        assumeNotNull(l.target_id)                                    AS issue_id,
        l.target_readable                                             AS id_readable,
        'sub_issue'                                                   AS link_type,
        'sub_issue'                                                   AS occurrence_key,
        l.issue_id                                                    AS target_ident,
        toNullable(l.issue_id)                                        AS target_id,
        l.id_readable                                                 AS target_readable,
        l.valid_from                                                  AS valid_from,
        l.valid_from_known                                            AS valid_from_known,
        l.valid_to                                                    AS valid_to,
        l.added_by                                                    AS added_by,
        l.removed_by                                                  AS removed_by,
        l.origin_event_id                                             AS origin_event_id,
        l.evidence                                                    AS evidence,
        l.collected_at                                                AS collected_at
    FROM lasting AS l
    WHERE l.link_type = 'parent'
      AND l.target_id IS NOT NULL
),

every_interval AS (
    SELECT
        insight_source_id, issue_id, id_readable, link_type, occurrence_key, target_ident, target_id, target_readable,
        valid_from, valid_from_known, valid_to, added_by, removed_by, origin_event_id, evidence, collected_at
    FROM lasting

    UNION ALL

    SELECT
        insight_source_id, issue_id, id_readable, link_type, occurrence_key, target_ident, target_id, target_readable,
        valid_from, valid_from_known, valid_to, added_by, removed_by, origin_event_id, evidence, collected_at
    FROM mirrored
)

SELECT
    CAST(concat(
        insight_source_id, '-jira-', issue_id, '-', link_type, '-', occurrence_key, '-',
        target_ident, '-', toString(toUnixTimestamp64Milli(valid_from))
    ) AS String)                                                      AS unique_key,
    CAST(insight_source_id AS String)                                 AS insight_source_id,
    CAST('jira' AS String)                                            AS data_source,
    CAST(id_readable AS String)                                       AS id_readable,
    CAST(link_type AS String)                                         AS link_type,
    CAST('issue' AS Enum8('issue' = 1, 'pull_request' = 2))           AS target_type,
    CAST(target_readable AS String)                                   AS target_readable,
    -- Jira project keys cannot contain '-', so the key prefix is the project.
    CAST(splitByChar('-', id_readable)[1] != splitByChar('-', target_readable)[1] AS Bool) AS is_cross_repository,
    toDateTime64(valid_from, 3)                                       AS valid_from,
    valid_from_known                                                  AS valid_from_known,
    CAST(valid_to AS Nullable(DateTime64(3)))                         AS valid_to,
    CAST(added_by AS Nullable(String))                                AS added_by,
    CAST(removed_by AS Nullable(String))                              AS removed_by,
    CAST(evidence AS Enum8('event' = 1, 'observation' = 2))           AS evidence,
    CAST(origin_event_id AS Nullable(String))                         AS origin_event_id,
    toDateTime64(collected_at, 3)                                     AS collected_at,
    CAST(toUnixTimestamp64Milli(toDateTime64(collected_at, 3)) AS UInt64) AS _version,
    CAST(issue_id AS Nullable(String))                                AS issue_id,
    CAST(target_id AS Nullable(String))                               AS target_id
FROM every_interval

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
-- An occurrence is one relation from one issue to one linked issue: "blocks
-- PROJ-2" and "relates to PROJ-2" are two, and each opens and closes on its own.
-- Jira logs a link on both issues, each in its own words, so each side is read
-- from its own changelog and nothing is mirrored. A parent change is logged on
-- the child alone, so its `sub_issue` side is mirrored onto the parent.
--
-- A link the issue held before its first event on that occurrence has no add:
-- it is the first event's removal, or the snapshot's link when no event touched
-- it. Its interval opens at the issue's creation, the earliest it can have
-- existed, with `valid_from_known = 0`.
--
-- `target_id` is the linked issue's immutable id: the snapshot states it, and a
-- changelog key resolves through `jira__issue_key_ids`. A key that resolves to
-- nothing keeps the occurrence apart by the key and leaves `target_id` NULL.

WITH key_ids AS (
    SELECT insight_source_id, issue_key, issue_id
    FROM {{ ref('jira__issue_key_ids') }} FINAL
),

held_now AS (
    SELECT
        insight_source_id, issue_id, id_readable, kind, type_name, type_outward, type_inward,
        relation, target_id, target_key, extracted_at
    FROM {{ ref('jira__issue_link_snapshot') }} FINAL
),

-- The issue's current key and its creation, from its newest bronze row.
issues AS (
    SELECT
        COALESCE(source_id, '')                                       AS insight_source_id,
        COALESCE(toString(jira_id), '')                               AS issue_id,
        argMax(COALESCE(toString(id_readable), ''), _airbyte_extracted_at) AS current_key,
        argMax(parseDateTime64BestEffortOrNull(created, 3), _airbyte_extracted_at) AS created_at
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

-- What each relation means, from the link types the snapshots carry. A link
-- object states both of its type's phrases, so a relation is known here while
-- any issue holds a link of its type, from either side.
relation_types AS (
    SELECT
        insight_source_id,
        relation,
        type_and_direction.1                                          AS type_name,
        type_and_direction.2                                          AS direction
    FROM (
        SELECT
            insight_source_id,
            phrase.1                                                  AS relation,
            min((type_name, phrase.2))                                AS type_and_direction
        FROM held_now
        ARRAY JOIN [(type_outward, 'outward'), (type_inward, 'inward')] AS phrase
        WHERE kind = 'issuelink' AND phrase.1 != ''
        GROUP BY insight_source_id, relation
    )
),

link_items AS (
    SELECT
        insight_source_id,
        assumeNotNull(jira_id)                                        AS issue_id,
        id_readable                                                   AS issue_key,
        changelog_id,
        created_at                                                    AS event_at,
        author_account_id                                             AS author_id,
        extracted_at,
        if(COALESCE(value_to, value_to_string, '') != '', 'add', 'remove') AS action,
        if(action = 'add', COALESCE(value_to, ''), COALESCE(value_from, ''))               AS target_key,
        if(action = 'add', COALESCE(value_to_string, ''), COALESCE(value_from_string, '')) AS sentence
    FROM {{ ref('jira__changelog_items') }}
    WHERE field_id = 'issuelinks'
      AND jira_id IS NOT NULL
),

-- One operation per element an event added or removed. A remote link is logged
-- in `issuelinks` under its own all-digit id and is no issue link (FIELD-
-- HISTORY-IN-DBT.md §3.7). A parent change removes the old parent and adds the
-- new one, each named by id with its key of the time.
operations AS (
    SELECT
        insight_source_id, issue_id, issue_key, changelog_id, event_at, author_id, extracted_at,
        action,
        'issuelink'                                                   AS kind,
        {{ jira_link_relation('sentence', 'target_key') }}            AS relation,
        ''                                                            AS target_id,
        target_key
    FROM link_items
    WHERE target_key != ''
      AND NOT match(target_key, '^[0-9]+$')

    UNION ALL

    SELECT
        insight_source_id,
        assumeNotNull(jira_id)                                        AS issue_id,
        id_readable                                                   AS issue_key,
        changelog_id,
        created_at                                                    AS event_at,
        author_account_id                                             AS author_id,
        extracted_at,
        side.1                                                        AS action,
        'parent'                                                      AS kind,
        'parent'                                                      AS relation,
        side.2                                                        AS target_id,
        side.3                                                        AS target_key
    FROM {{ ref('jira__changelog_items') }}
    ARRAY JOIN [
        ('remove', COALESCE(value_from, ''), COALESCE(value_from_string, '')),
        ('add', COALESCE(value_to, ''), COALESCE(value_to_string, ''))
    ] AS side
    WHERE field_id = 'parent'
      AND jira_id IS NOT NULL
      AND (side.2 != '' OR side.3 != '')
),

resolved_operations AS (
    SELECT
        o.* EXCEPT (target_id),
        COALESCE(nullIf(o.target_id, ''), nullIf(COALESCE(k.issue_id, ''), ''), '') AS resolved_id,
        resolved_id                                                   AS target_id,
        if(resolved_id != '', resolved_id, o.target_key)              AS target_ident
    FROM operations AS o
    LEFT JOIN key_ids AS k
        ON k.insight_source_id = o.insight_source_id
       AND k.issue_key = o.target_key
),

-- An occurrence's identity, and its operations in the order they happened.
occurrence_operations AS (
    SELECT
        insight_source_id,
        issue_id,
        kind,
        relation,
        target_ident,
        argMax(issue_key, (event_at, toUInt64OrZero(changelog_id)))   AS issue_key,
        any(target_id)                                                AS target_id,
        argMax(target_key, (event_at, toUInt64OrZero(changelog_id)))  AS target_key,
        arraySort(o -> (o.1, o.2),
                  groupArray((event_at, toUInt64OrZero(changelog_id), action, author_id,
                              changelog_id, extracted_at)))           AS ops
    FROM resolved_operations
    GROUP BY insight_source_id, issue_id, kind, relation, target_ident
),

held_occurrences AS (
    SELECT
        h.insight_source_id                                           AS insight_source_id,
        h.issue_id                                                    AS issue_id,
        h.kind                                                        AS kind,
        h.relation                                                    AS relation,
        if(resolved_id != '', resolved_id, h.target_key)              AS target_ident,
        h.id_readable                                                 AS issue_key,
        COALESCE(nullIf(h.target_id, ''), nullIf(COALESCE(k.issue_id, ''), ''), '') AS resolved_id,
        h.target_key                                                  AS target_key,
        h.extracted_at                                                AS extracted_at
    FROM held_now AS h
    LEFT JOIN key_ids AS k
        ON k.insight_source_id = h.insight_source_id
       AND k.issue_key = h.target_key
),

-- Every occurrence, with its operations (none when only the snapshot holds it)
-- and whether it was held before them.
--
-- The state after an operation is the operation itself: present after an add,
-- absent after a remove. An occurrence was held before its first operation
-- when that operation removed it, and before every operation when it has none.
-- An add while present and a remove while absent repeat what is already true.
occurrences AS (
    SELECT
        COALESCE(o.insight_source_id, h.insight_source_id)            AS insight_source_id,
        COALESCE(o.issue_id, h.issue_id)                              AS issue_id,
        COALESCE(o.kind, h.kind)                                      AS kind,
        COALESCE(o.relation, h.relation)                              AS relation,
        COALESCE(o.target_ident, h.target_ident)                      AS target_ident,
        COALESCE(nullIf(h.issue_key, ''), o.issue_key, '')            AS issue_key,
        COALESCE(nullIf(o.target_id, ''), nullIf(h.resolved_id, ''), '') AS target_id,
        COALESCE(nullIf(h.target_key, ''), o.target_key, '')          AS target_key,
        COALESCE(o.ops, [])                                           AS ops,
        h.extracted_at                                                AS held_extracted_at,
        empty(ops) OR ops[1].3 = 'remove'                             AS held_before
    FROM occurrence_operations AS o
    FULL OUTER JOIN held_occurrences AS h
        ON h.insight_source_id = o.insight_source_id
       AND h.issue_id = o.issue_id
       AND h.kind = o.kind
       AND h.relation = o.relation
       AND h.target_ident = o.target_ident
),

-- Positions in `ops` where the occurrence appears and disappears; 0 is "held
-- before the first operation". They alternate by construction, so the n-th
-- opening pairs with the n-th closing.
transitions AS (
    SELECT
        *,
        arrayMap(k -> if(k = 1, held_before, ops[k - 1].3 = 'add'), arrayEnumerate(ops)) AS present_before,
        arrayConcat(if(held_before, [0], []),
                    arrayFilter(k -> ops[k].3 = 'add' AND NOT present_before[k], arrayEnumerate(ops))) AS openings,
        arrayFilter(k -> ops[k].3 = 'remove' AND present_before[k], arrayEnumerate(ops)) AS closings
    FROM occurrences
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
           {{ jira_link_type("COALESCE(rt.type_name, '')", "COALESCE(rt.direction, '')", 'i.relation') }}) AS link_type,
        i.relation                                                    AS relation,
        i.target_ident                                                AS target_ident,
        nullIf(i.target_id, '')                                       AS target_id,
        COALESCE(nullIf(COALESCE(target.current_key, ''), ''),
                 nullIf(COALESCE(linked.current_key, ''), ''),
                 i.target_key)                                        AS target_readable,
        -- No creation date means no issue row to read it from; the epoch then
        -- says "unknown", as `valid_from_known = 0` lets it.
        if(i.opened_by = 0, COALESCE(owner.created_at, toDateTime64(0, 3)), i.ops[i.opened_by].1) AS valid_from,
        toUInt8(i.opened_by != 0)                                     AS valid_from_known,
        if(i.closed_by = 0, CAST(NULL AS Nullable(DateTime64(3))), toNullable(i.ops[i.closed_by].1)) AS valid_to,
        if(i.opened_by = 0, CAST(NULL AS Nullable(String)), i.ops[i.opened_by].4) AS added_by,
        if(i.closed_by = 0, CAST(NULL AS Nullable(String)), i.ops[i.closed_by].4) AS removed_by,
        if(i.opened_by = 0, CAST(NULL AS Nullable(String)), toNullable(i.ops[i.opened_by].5)) AS origin_event_id,
        -- INVARIANT: the newest evidence behind the row, so an interval that
        -- only CLOSED still moves `_version`.
        greatest(
            if(i.opened_by = 0, COALESCE(i.held_extracted_at, toDateTime64(0, 3)), i.ops[i.opened_by].6),
            if(i.closed_by = 0, toDateTime64(0, 3), i.ops[i.closed_by].6)
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
    LEFT JOIN relation_types AS rt
        ON rt.insight_source_id = i.insight_source_id
       AND rt.relation = i.relation
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
        'sub_issue'                                                   AS relation,
        l.issue_id                                                    AS target_ident,
        toNullable(l.issue_id)                                        AS target_id,
        l.id_readable                                                 AS target_readable,
        l.valid_from                                                  AS valid_from,
        l.valid_from_known                                            AS valid_from_known,
        l.valid_to                                                    AS valid_to,
        l.added_by                                                    AS added_by,
        l.removed_by                                                  AS removed_by,
        l.origin_event_id                                             AS origin_event_id,
        l.collected_at                                                AS collected_at
    FROM lasting AS l
    WHERE l.link_type = 'parent'
      AND l.target_id IS NOT NULL
),

every_interval AS (
    SELECT
        insight_source_id, issue_id, id_readable, link_type, relation, target_ident, target_id, target_readable,
        valid_from, valid_from_known, valid_to, added_by, removed_by, origin_event_id, collected_at
    FROM lasting

    UNION ALL

    SELECT
        insight_source_id, issue_id, id_readable, link_type, relation, target_ident, target_id, target_readable,
        valid_from, valid_from_known, valid_to, added_by, removed_by, origin_event_id, collected_at
    FROM mirrored
)

SELECT
    CAST(concat(
        insight_source_id, '-jira-', issue_id, '-', link_type, '-', relation, '-',
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
    CAST('event' AS Enum8('event' = 1, 'observation' = 2))            AS evidence,
    CAST(origin_event_id AS Nullable(String))                         AS origin_event_id,
    toDateTime64(collected_at, 3)                                     AS collected_at,
    CAST(toUnixTimestamp64Milli(toDateTime64(collected_at, 3)) AS UInt64) AS _version,
    CAST(issue_id AS Nullable(String))                                AS issue_id,
    CAST(target_id AS Nullable(String))                               AS target_id
FROM every_interval

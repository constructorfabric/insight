{{ config(
    materialized='table',
    alias='jira__link_occurrences',
    schema='staging',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    query_settings={'join_use_nulls': 1},
    tags=['jira', 'staging']
) }}

-- Every Jira link occurrence an issue has had, with the changelog operations
-- that opened and closed it and whether it was held before them. Read by
-- `jira__task_links`, which folds the operations into intervals.
--
-- An occurrence is one link of one type from one issue to one linked issue:
-- "blocks PROJ-2" and "relates to PROJ-2" are two. The changelog names a link's
-- type only by the phrase the type had that day, and a type can be reworded, so
-- a phrase is resolved to the type's immutable id: the link held between the
-- same two issues first, then the type the phrase has been seen to mean. A
-- phrase that resolves to no type keeps its occurrence apart by the phrase.
--
-- `target_ident` is the linked issue's immutable id: the snapshot states it,
-- and a changelog key resolves through `jira__issue_key_ids`. A key that
-- resolves to nothing keeps the occurrence apart by the key.

WITH key_ids AS (
    SELECT tenant_id, insight_source_id, issue_key, issue_id
    FROM {{ ref('jira__issue_key_ids') }} FINAL
),

-- The fields an epic's children name it in. Before Jira made the epic the
-- issue's `parent`, it was set through this field, and its changelog items name
-- the epic by id and key exactly as a parent change does.
epic_link_fields AS (
    SELECT DISTINCT
        ifNull(tenant_id, '')                                         AS tenant_id,
        CAST(source_id AS String)                                     AS insight_source_id,
        field_id
    FROM {{ source('bronze_jira', 'jira_fields') }} FINAL
    WHERE schema_custom = 'com.pyxis.greenhopper.jira:gh-epic-link'
),

held_now AS (
    SELECT
        tenant_id, insight_source_id, issue_id, id_readable, kind, type_id, direction,
        relation, target_id, target_key, extracted_at
    FROM {{ ref('jira__issue_link_snapshot') }} FINAL
),

link_items AS (
    SELECT
        tenant_id,
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
-- new one, each named by id with its key of the time; an Epic Link change is
-- one too.
operations AS (
    SELECT
        tenant_id, insight_source_id, issue_id, issue_key, changelog_id, event_at, author_id, extracted_at,
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
        tenant_id,
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
    WHERE (field_id = 'parent'
           OR (tenant_id, insight_source_id, field_id) IN (SELECT tenant_id, insight_source_id, field_id FROM epic_link_fields))
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
        ON k.tenant_id = o.tenant_id
       AND k.insight_source_id = o.insight_source_id
       AND k.issue_key = o.target_key
),

-- The operations each phrase logged against one linked issue, in order.
phrase_occurrences AS (
    SELECT
        tenant_id,
        insight_source_id,
        issue_id,
        kind,
        relation,
        target_ident,
        argMax(issue_key, (event_at, toUInt64OrZero(changelog_id)))   AS issue_key,
        any(target_id)                                                AS target_id,
        argMax(target_key, (event_at, toUInt64OrZero(changelog_id)))  AS target_key,
        arraySort(o -> (o.1, o.2, o.3 = 'add'),
                  groupArray((event_at, toUInt64OrZero(changelog_id), action, author_id,
                              changelog_id, extracted_at)))           AS phrase_ops
    FROM resolved_operations
    GROUP BY tenant_id, insight_source_id, issue_id, kind, relation, target_ident
),

held_occurrences AS (
    SELECT
        h.tenant_id                                                   AS tenant_id,
        h.insight_source_id                                           AS insight_source_id,
        h.issue_id                                                    AS issue_id,
        h.kind                                                        AS kind,
        h.relation                                                    AS relation,
        if(h.kind = 'parent', 'parent', concat('type:', h.type_id, ':', h.direction)) AS occurrence_key,
        if(resolved_id != '', resolved_id, h.target_key)              AS target_ident,
        h.id_readable                                                 AS issue_key,
        COALESCE(nullIf(h.target_id, ''), nullIf(COALESCE(k.issue_id, ''), ''), '') AS resolved_id,
        h.target_key                                                  AS target_key,
        h.extracted_at                                                AS extracted_at
    FROM held_now AS h
    LEFT JOIN key_ids AS k
        ON k.tenant_id = h.tenant_id
       AND k.insight_source_id = h.insight_source_id
       AND k.issue_key = h.target_key
),

-- A phrase the link held between the same two issues states as its own.
stated AS (
    SELECT
        p.tenant_id                                                   AS tenant_id,
        p.insight_source_id                                           AS insight_source_id,
        p.issue_id                                                    AS issue_id,
        p.relation                                                    AS relation,
        p.target_ident                                                AS target_ident,
        min(h.occurrence_key)                                         AS occurrence_key,
        any(p.phrase_ops[1].3 = 'remove')                             AS held_before_phrase
    FROM phrase_occurrences AS p
    INNER JOIN held_occurrences AS h
        ON h.tenant_id = p.tenant_id
       AND h.insight_source_id = p.insight_source_id
       AND h.issue_id = p.issue_id
       AND h.kind = p.kind
       AND h.target_ident = p.target_ident
       AND h.relation = p.relation
    WHERE p.kind = 'issuelink'
    GROUP BY p.tenant_id, p.insight_source_id, p.issue_id, p.relation, p.target_ident
),

-- Between two issues, the phrases still added at their last event that no held
-- link states, and the held links no phrase opened: unstated, or stated by a
-- phrase whose first event removed it. One of each is that link under a phrase
-- its type has since dropped.
open_unstated AS (
    SELECT
        p.tenant_id                                                   AS tenant_id,
        p.insight_source_id                                           AS insight_source_id,
        p.issue_id                                                    AS issue_id,
        p.target_ident                                                AS target_ident,
        groupArray(p.relation)                                        AS phrases
    FROM phrase_occurrences AS p
    LEFT ANTI JOIN stated AS s
        ON s.tenant_id = p.tenant_id
       AND s.insight_source_id = p.insight_source_id
       AND s.issue_id = p.issue_id
       AND s.relation = p.relation
       AND s.target_ident = p.target_ident
    WHERE p.kind = 'issuelink'
      AND p.phrase_ops[-1].3 = 'add'
    GROUP BY p.tenant_id, p.insight_source_id, p.issue_id, p.target_ident
),

unclaimed_held AS (
    SELECT
        h.tenant_id                                                   AS tenant_id,
        h.insight_source_id                                           AS insight_source_id,
        h.issue_id                                                    AS issue_id,
        h.target_ident                                                AS target_ident,
        groupArray(h.occurrence_key)                                  AS occurrence_keys
    FROM held_occurrences AS h
    LEFT ANTI JOIN (SELECT * FROM stated WHERE NOT held_before_phrase) AS s
        ON s.tenant_id = h.tenant_id
       AND s.insight_source_id = h.insight_source_id
       AND s.issue_id = h.issue_id
       AND s.target_ident = h.target_ident
       AND s.occurrence_key = h.occurrence_key
    WHERE h.kind = 'issuelink'
    GROUP BY h.tenant_id, h.insight_source_id, h.issue_id, h.target_ident
),

paired AS (
    SELECT
        o.tenant_id                                                   AS tenant_id,
        o.insight_source_id                                           AS insight_source_id,
        o.issue_id                                                    AS issue_id,
        o.target_ident                                                AS target_ident,
        o.phrases[1]                                                  AS relation,
        u.occurrence_keys[1]                                          AS occurrence_key
    FROM open_unstated AS o
    INNER JOIN unclaimed_held AS u
        ON u.tenant_id = o.tenant_id
       AND u.insight_source_id = o.insight_source_id
       AND u.issue_id = o.issue_id
       AND u.target_ident = o.target_ident
    WHERE length(o.phrases) = 1
      AND length(u.occurrence_keys) = 1
),

matched AS (
    SELECT tenant_id, insight_source_id, issue_id, relation, target_ident, occurrence_key FROM stated
    UNION ALL
    SELECT tenant_id, insight_source_id, issue_id, relation, target_ident, occurrence_key FROM paired
),

-- Every type a phrase has been seen to mean, most often first: the phrases the
-- held links state, and the dropped ones matched to a held link above.
phrase_meanings AS (
    SELECT
        tenant_id,
        insight_source_id,
        relation,
        arrayMap(m -> m.2, arraySort(m -> (-toInt64(m.1), m.2),
                                     groupArray((seen, occurrence_key)))) AS occurrence_keys
    FROM (
        SELECT tenant_id, insight_source_id, relation, occurrence_key, count() AS seen
        FROM (
            SELECT tenant_id, insight_source_id, relation, occurrence_key FROM held_occurrences WHERE kind = 'issuelink'
            UNION ALL
            SELECT tenant_id, insight_source_id, relation, occurrence_key FROM paired
        )
        GROUP BY tenant_id, insight_source_id, relation, occurrence_key
    )
    GROUP BY tenant_id, insight_source_id, relation
),

phrase_candidates AS (
    SELECT
        p.tenant_id                                                   AS tenant_id,
        p.insight_source_id                                           AS insight_source_id,
        p.issue_id                                                    AS issue_id,
        p.kind                                                        AS kind,
        p.relation                                                    AS relation,
        p.target_ident                                                AS target_ident,
        p.issue_key                                                   AS issue_key,
        p.target_id                                                   AS target_id,
        p.target_key                                                  AS target_key,
        p.phrase_ops                                                  AS phrase_ops,
        COALESCE(m.occurrence_key, '')                                AS matched_key,
        pm.occurrence_keys                                            AS meanings
    FROM phrase_occurrences AS p
    LEFT JOIN matched AS m
        ON m.tenant_id = p.tenant_id
       AND m.insight_source_id = p.insight_source_id
       AND m.issue_id = p.issue_id
       AND m.relation = p.relation
       AND m.target_ident = p.target_ident
    LEFT JOIN phrase_meanings AS pm
        ON pm.tenant_id = p.tenant_id
       AND pm.insight_source_id = p.insight_source_id
       AND pm.relation = p.relation
),

-- The types known between two issues: the links held there and the phrases
-- that resolve without a choice.
pair_keys AS (
    SELECT
        tenant_id,
        insight_source_id,
        issue_id,
        target_ident,
        groupUniqArray(occurrence_key)                                AS occurrence_keys
    FROM (
        SELECT tenant_id, insight_source_id, issue_id, target_ident, occurrence_key
        FROM held_occurrences
        WHERE kind = 'issuelink'

        UNION ALL

        SELECT tenant_id, insight_source_id, issue_id, target_ident, if(matched_key != '', matched_key, meanings[1])
        FROM phrase_candidates
        WHERE kind = 'issuelink'
          AND (matched_key != '' OR length(meanings) = 1)
    )
    GROUP BY tenant_id, insight_source_id, issue_id, target_ident
),

-- A phrase's type: the held link it was matched to; else, of the types the
-- phrase has meant, one already known between the same two issues; else the
-- one it means most often.
keyed_phrases AS (
    SELECT
        c.tenant_id                                                   AS tenant_id,
        c.insight_source_id                                           AS insight_source_id,
        c.issue_id                                                    AS issue_id,
        c.kind                                                        AS kind,
        c.target_ident                                                AS target_ident,
        c.issue_key                                                   AS issue_key,
        c.target_id                                                   AS target_id,
        c.target_key                                                  AS target_key,
        c.phrase_ops                                                  AS phrase_ops,
        arrayFilter(k -> has(pk.occurrence_keys, k), c.meanings)      AS meanings_here,
        multiIf(
            c.kind = 'parent',      'parent',
            c.matched_key != '',    c.matched_key,
            notEmpty(meanings_here), meanings_here[1],
            notEmpty(c.meanings),   c.meanings[1],
            concat('phrase:', c.relation)
        )                                                             AS occurrence_key
    FROM phrase_candidates AS c
    LEFT JOIN pair_keys AS pk
        ON pk.tenant_id = c.tenant_id
       AND pk.insight_source_id = c.insight_source_id
       AND pk.issue_id = c.issue_id
       AND pk.target_ident = c.target_ident
),

-- An occurrence's identity, and its operations in the order they happened,
-- whichever phrase logged them. One entry that removes and adds the same link
-- ("from X to X") leaves it held: its removal sorts first.
occurrence_operations AS (
    SELECT
        tenant_id,
        insight_source_id,
        issue_id,
        kind,
        occurrence_key,
        target_ident,
        argMax(issue_key, phrase_ops[-1].1)                           AS issue_key,
        any(target_id)                                                AS target_id,
        argMax(target_key, phrase_ops[-1].1)                          AS target_key,
        arraySort(o -> (o.1, o.2, o.3 = 'add'), arrayFlatten(groupArray(phrase_ops))) AS ops
    FROM keyed_phrases
    GROUP BY tenant_id, insight_source_id, issue_id, kind, occurrence_key, target_ident
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
        CAST(COALESCE(o.tenant_id, h.tenant_id) AS String)            AS tenant_id,
        COALESCE(o.insight_source_id, h.insight_source_id)            AS insight_source_id,
        COALESCE(o.issue_id, h.issue_id)                              AS issue_id,
        COALESCE(o.kind, h.kind)                                      AS kind,
        COALESCE(o.occurrence_key, h.occurrence_key, '')              AS occurrence_key,
        if(startsWith(occurrence_key, 'type:'), splitByChar(':', occurrence_key)[2], '') AS type_id,
        if(startsWith(occurrence_key, 'type:'), splitByChar(':', occurrence_key)[3], '') AS direction,
        if(startsWith(occurrence_key, 'phrase:'), substring(occurrence_key, 8), '')     AS phrase,
        COALESCE(o.target_ident, h.target_ident)                      AS target_ident,
        COALESCE(nullIf(h.issue_key, ''), o.issue_key, '')            AS issue_key,
        COALESCE(nullIf(o.target_id, ''), nullIf(h.resolved_id, ''), '') AS target_id,
        COALESCE(nullIf(h.target_key, ''), o.target_key, '')          AS target_key,
        COALESCE(o.ops, [])                                           AS ops,
        h.extracted_at                                                AS held_extracted_at,
        empty(ops) OR ops[1].3 = 'remove'                             AS held_before
    FROM occurrence_operations AS o
    FULL OUTER JOIN held_occurrences AS h
        ON h.tenant_id = o.tenant_id
       AND h.insight_source_id = o.insight_source_id
       AND h.issue_id = o.issue_id
       AND h.kind = o.kind
       AND h.occurrence_key = o.occurrence_key
       AND h.target_ident = o.target_ident
)

SELECT
    CAST(concat(
        ifNull(tenant_id, ''), '-', insight_source_id, '-', issue_id, '-', kind, '-', occurrence_key, '-', target_ident
    ) AS String)                                                      AS unique_key,
    tenant_id,
    insight_source_id,
    issue_id,
    kind,
    occurrence_key,
    type_id,
    direction,
    phrase,
    target_ident,
    issue_key,
    target_id,
    target_key,
    ops,
    held_extracted_at,
    held_before,
    toUnixTimestamp64Milli(now64(3))                                  AS _version
FROM occurrences

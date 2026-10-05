{#
  The element-wise stage of the field-history journal
  (`jira_journal_derive_stages`): one row per operation on a list field, with
  the state it produced and the field's set at creation.
#}


{% macro jira_journal_element_wise_state_sql() %}
WITH kinds AS (
    SELECT * FROM {{ jira_journal_stage('kinds') }}
),

ranked_events AS (
    SELECT * FROM {{ jira_journal_stage('ranked_events') }}
),

-- Current value per (issue, field), the seed for the backward reconstruction.
--
-- Two projections of the same relation on purpose. Only the element-wise kinds need
-- the seed at all (§2.1), and that subset is a small fraction of the snapshot —
-- joining the whole thing builds a hash table over every field of every issue
-- for no benefit.
snapshot_element_wise AS (
    SELECT
        s.insight_source_id                               AS insight_source_id,
        s.issue_id                                        AS issue_id,
        s.field_id                                        AS field_id,
        s.value_ids                                       AS value_ids,
        s.value_displays                                  AS value_displays
    FROM {{ ref('jira__issue_field_snapshot') }} AS s FINAL
    INNER JOIN kinds AS k
        ON k.insight_source_id = s.insight_source_id
       AND k.field_id = s.field_id
    WHERE k.field_kind IN {{ jira_element_wise_kinds() }}
      AND {{ jira_journal_in_batch('s.insight_source_id', 's.issue_id') }}
),


-- ── the element-wise kinds, whose state accumulates ────────────────────────
-- Elements are carried as one string per element so ids and displays cannot
-- drift apart; they are split back into the parallel arrays at the end.
--
-- The state is derived PER ELEMENT, never by carrying a running list. An
-- element belongs to the state after event k exactly when its own latest
-- operation at or before k was an `add`; with no operation of its own by then,
-- it belongs there exactly when it belonged at creation. Each element's
-- ordered operations therefore become the SPANS of events over which it is
-- present, and the state after event k is every element whose span covers k.
--
-- Why per element and not the set arithmetic this replaced. The closed form
-- `(initial ∪ additions up to k) \ removals up to k` is wrong for an element
-- added, removed and ADDED AGAIN: it stays subtracted forever, because it is
-- in "every removal". Reading the element's LATEST operation is correct for
-- any cycle — which is what a sequential fold bought, without its cost.
--
-- COST (§13): a running list costs O(n^2) in the number of operations on one
-- (issue, field), either by replaying the list per row or by appending to an
-- accumulator — `arrayPushBack` copies the whole accumulator, so appending n
-- states copies n^2/2 of them. A long-lived list field can accumulate enough
-- events on a single issue to exhaust any memory limit that way, and no
-- partitioning of the input helps: one (issue, field) is one sequence and
-- cannot be split. Spans cost the size of the output and nothing beyond it.
--
-- Presence at creation needs no snapshot: an element whose FIRST operation
-- removed it was there, one whose first operation added it was not. The
-- snapshot supplies only the elements no operation ever touched.
element_wise_items AS (
    SELECT
        e.insight_source_id                               AS insight_source_id,
        e.issue_id                                        AS issue_id,
        e.field_id                                        AS field_id,
        e.field_name                                      AS field_name,
        e.field_kind                                      AS field_kind,
        e.changelog_id                                    AS changelog_id,
        e.event_at                                        AS event_at,
        e.author_id                                       AS author_id,
        e.delta_action                                    AS delta_action,
        e.entry_rank                                      AS entry_rank,
        e.element.1                                       AS element_id,
        concat(e.element.1, '\x1f', e.element.2)           AS pair,
        -- The event's position in this (issue, field)'s sequence, entries in
        -- the order of their instant. Items of one ENTRY share (event_at,
        -- entry_rank, event_ord); the element id breaks that tie so
        -- the numbering is reproducible where the window's order among them was
        -- arbitrary. Items of one entry name distinct elements, so no element's
        -- own order depends on the tiebreak.
        row_number() OVER (PARTITION BY e.insight_source_id, e.issue_id, e.field_id
                           ORDER BY e.event_at, e.entry_rank, e.event_ord, e.element.1) AS seq
    FROM ranked_events AS e
    WHERE e.field_kind IN {{ jira_element_wise_kinds() }}
),

element_wise_extent AS (
    SELECT
        insight_source_id                                 AS insight_source_id,
        issue_id                                          AS issue_id,
        field_id                                          AS field_id,
        max(seq)                                          AS last_seq
    FROM element_wise_items
    GROUP BY insight_source_id, issue_id, field_id
),

-- One row per (issue, field, element), carrying that element's own operations.
element_wise_element AS (
    SELECT
        i.insight_source_id                               AS insight_source_id,
        i.issue_id                                        AS issue_id,
        i.field_id                                        AS field_id,
        i.element_id                                      AS element_id,
        argMin(i.pair, i.seq)                             AS first_pair,
        argMin(i.delta_action, i.seq)                     AS first_action,
        min(i.seq)                                        AS first_seq,
        arraySort(x -> x.1,
                  groupArray((i.seq, i.delta_action, i.pair)))  AS ops
    FROM element_wise_items AS i
    GROUP BY i.insight_source_id, i.issue_id, i.field_id, i.element_id
),

-- The snapshot's elements, deduplicated by id exactly as the pairs were.
element_wise_snapshot_pairs AS (
    SELECT
        s.insight_source_id                               AS insight_source_id,
        s.issue_id                                        AS issue_id,
        s.field_id                                        AS field_id,
        splitByChar('\x1f', s.pair)[1]                    AS element_id,
        s.pair                                            AS pair
    FROM (
        SELECT
            insight_source_id,
            issue_id,
            field_id,
            arrayJoin({{ jira_distinct_pairs_by_id("arrayMap(j -> concat(value_ids[j], '\x1f', value_displays[j]), range(1, length(value_ids) + 1))") }}) AS pair
        FROM snapshot_element_wise
    ) AS s
),

-- Elements the log never touched. They are present throughout, so they belong
-- to the state after every event as well as to the state at creation.
element_wise_untouched AS (
    SELECT
        p.insight_source_id                               AS insight_source_id,
        p.issue_id                                        AS issue_id,
        p.field_id                                        AS field_id,
        p.pair                                            AS pair
    FROM element_wise_snapshot_pairs AS p
    LEFT ANTI JOIN element_wise_element AS e
        ON e.insight_source_id = p.insight_source_id
       AND e.issue_id = p.issue_id
       AND e.field_id = p.field_id
       AND e.element_id = p.element_id
),

-- (first event of the span, last event of the span, the pair to carry). An
-- `add` opens a span that runs until this element's NEXT operation; presence at
-- creation opens one that runs until its FIRST. A `remove` opens nothing.
element_wise_spans AS (
    SELECT
        e.insight_source_id                               AS insight_source_id,
        e.issue_id                                        AS issue_id,
        e.field_id                                        AS field_id,
        arrayJoin(arrayConcat(
            if(e.first_action = 'remove' AND e.first_seq > 1,
               [(toUInt64(0), toUInt64(e.first_seq - 1), e.first_pair)],
               CAST([] AS Array(Tuple(UInt64, UInt64, String)))),
            arrayMap(k -> (e.ops[k].1,
                           if(k = length(e.ops), x.last_seq, toUInt64(e.ops[k + 1].1 - 1)),
                           e.ops[k].3),
                     arrayFilter(k -> e.ops[k].2 = 'add', arrayEnumerate(e.ops)))
        ))                                                AS span
    FROM element_wise_element AS e
    INNER JOIN element_wise_extent AS x
        ON x.insight_source_id = e.insight_source_id
       AND x.issue_id = e.issue_id
       AND x.field_id = e.field_id

    UNION ALL

    SELECT
        u.insight_source_id                               AS insight_source_id,
        u.issue_id                                        AS issue_id,
        u.field_id                                        AS field_id,
        -- 0, not 1: an element the log never touched was in the list before any
        -- event, so it orders ahead of one added by the first event.
        (toUInt64(0), x.last_seq, u.pair)                 AS span
    FROM element_wise_untouched AS u
    INNER JOIN element_wise_extent AS x
        ON x.insight_source_id = u.insight_source_id
       AND x.issue_id = u.issue_id
       AND x.field_id = u.field_id
),

-- The state after each event, as the elements whose span covers it, ordered by
-- when each element ENTERED the list: elements present at creation first, then
-- each addition in event order, and a re-added element at the back because its
-- new span starts later. That is the order a running list produced, and
-- `test_multi_elementwise` asserts it.
--
-- WITHIN the creation-time group the order is by element id. A running list
-- left that group in the reverse order of its first operations — an artefact of
-- rewinding the snapshot rather than a property of the data — so this group is
-- where the two differ; membership is the same either way, and the class
-- contract reads these arrays as a set.
element_wise_states AS (
    SELECT
        insight_source_id                                 AS insight_source_id,
        issue_id                                          AS issue_id,
        field_id                                          AS field_id,
        seq                                               AS seq,
        arrayMap(x -> x.2,
                 arraySort(x -> (x.1, x.2),
                           groupArray((span_start, pair)))) AS state_pairs
    FROM (
        SELECT
            insight_source_id,
            issue_id,
            field_id,
            arrayJoin(range(greatest(span.1, toUInt64(1)),
                            toUInt64(span.2 + 1)))        AS seq,
            span.1                                        AS span_start,
            span.3                                        AS pair
        FROM element_wise_spans
    )
    GROUP BY insight_source_id, issue_id, field_id, seq
),

element_wise_initial AS (
    SELECT
        insight_source_id                                 AS insight_source_id,
        issue_id                                          AS issue_id,
        field_id                                          AS field_id,
        arrayMap(x -> x.2,
                 arraySort(x -> (x.1, x.2),
                           groupArray((entered, pair))))  AS initial_pairs
    FROM (
        -- Same rule as the states: untouched elements were there before any
        -- event, an element whose first operation removed it was there too but
        -- is named by that operation.
        SELECT
            insight_source_id, issue_id, field_id,
            toUInt8(1)                                    AS entered,
            first_pair                                    AS pair
        FROM element_wise_element
        WHERE first_action = 'remove'

        UNION ALL

        SELECT insight_source_id, issue_id, field_id, toUInt8(0) AS entered, pair
        FROM element_wise_untouched
    )
    GROUP BY insight_source_id, issue_id, field_id
),

-- One row per operation again, with the state that operation produced.
-- `ops_seq` is its position in the pair's sequence, which is what the changelog
-- rows use to pick the state after the last item of an entry.
element_wise_state AS (
    SELECT
        i.insight_source_id                               AS insight_source_id,
        i.issue_id                                        AS issue_id,
        i.field_id                                        AS field_id,
        i.field_name                                      AS field_name,
        i.field_kind                                      AS field_kind,
        COALESCE(ini.initial_pairs, CAST([] AS Array(String)))  AS initial_pairs,
        i.changelog_id                                    AS changelog_id,
        i.event_at                                        AS event_at,
        i.author_id                                       AS author_id,
        i.delta_action                                    AS delta_action,
        i.entry_rank                                      AS entry_rank,
        COALESCE(st.state_pairs, CAST([] AS Array(String)))     AS state_pairs,
        i.seq                                             AS ops_seq
    FROM element_wise_items AS i
    LEFT JOIN element_wise_states AS st
        ON st.insight_source_id = i.insight_source_id
       AND st.issue_id = i.issue_id
       AND st.field_id = i.field_id
       AND st.seq = i.seq
    LEFT JOIN element_wise_initial AS ini
        ON ini.insight_source_id = i.insight_source_id
       AND ini.issue_id = i.issue_id
       AND ini.field_id = i.field_id
)

SELECT * FROM element_wise_state
{% endmacro %}

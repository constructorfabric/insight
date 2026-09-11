-- depends_on: {{ ref('jira__bronze_promoted') }}
-- depends_on: {{ ref('jira__task_field_kind') }}
-- depends_on: {{ ref('jira__issue_field_snapshot') }}
-- depends_on: {{ ref('jira__changelog_items') }}
{{ config(
    materialized='table',
    alias='jira__field_history_derived',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    query_settings={
        'max_bytes_before_external_group_by': 2000000000,
        'max_bytes_before_external_sort': 2000000000,
    },
    tags=['staging', 'jira']
) }}

-- The per-(issue x field x event) journal, derived in dbt.
--
-- This is the replacement for the Rust `jira-enrich` output. It is materialized
-- under its OWN name rather than over `staging.jira__task_field_history` so the
-- two can be compared row for row on a real warehouse before the binary is
-- retired; the cutover is a rename plus dropping the Rust arm from
-- `class_task_field_history`. See
-- `connectors/task-tracking/jira/specs/FIELD-HISTORY-IN-DBT.md`.
--
-- Six kinds of row, matching the contract the class consumers rely on (§10):
--   1. one creation marker per issue (`field_id = 'created'`, `_seq = 0`);
--   2. one `synthetic_initial` per (issue, field) holding the value at creation;
--   3. one `changelog` row per event, holding the state after that event;
--   4. one `retired_field` row per (issue, field) the issue stopped carrying;
--   5. one `unclassified_field` row per (issue, field) the catalogue lacks;
--   6. one `snapshot_diff` per (issue, field) the issue cleared without an
--      event — the state as OBSERVED, which is not the same claim as an event.
--
-- Only the element-wise kinds accumulate state across events; every other
-- kind's item carries both sides in full, so its rows are computed from the
-- item alone (§2.1). That is why there is no general fold here.

WITH kinds AS (
    SELECT
        insight_source_id,
        field_id,
        field_name,
        field_kind
    FROM {{ ref('jira__task_field_kind') }}
    -- `long_text` IS modelled: its body is content-addressed into
    -- `jira__task_field_text` and the journal carries the hash plus a prefix.
    WHERE field_kind NOT IN ('ignored', 'UNKNOWN')
      -- The catalogue contains a real `created` field, but `created` is also the
      -- contract's creation-marker sentinel (§10). Emitting both produces two
      -- rows with the SAME unique_key, and ReplacingMergeTree then keeps one and
      -- drops the other. The marker wins: its timestamp is the same value, and
      -- `task_issue_current_state.created_at` reads it by `event_kind`.
      AND field_id != 'created'
),

-- One row per issue: identity, creation time, reporter. Same two-pass dedup as
-- the snapshot model — the aggregation carries only a raw id, never the JSON.
issue_winner AS (
    -- Read-time dedup of the ReplacingMergeTree by the issue's stable key
    -- within a source, (source_id, jira_id): unmerged parts hold several rows
    -- per issue, and `unique_key` exists for the merge alone.
    SELECT source_id, jira_id, argMax(_airbyte_raw_id, _airbyte_extracted_at) AS raw_id
    FROM {{ source('bronze_jira', 'jira_issue') }}
    WHERE jira_id IS NOT NULL
    GROUP BY source_id, jira_id
),

issues AS (
    SELECT
        COALESCE(i.source_id, '')                         AS insight_source_id,
        COALESCE(toString(i.jira_id), '')                 AS issue_id,
        COALESCE(toString(i.id_readable), '')             AS id_readable,
        COALESCE(parseDateTime64BestEffortOrNull(i.created, 3),
                 toDateTime64(0, 3))                      AS created_at,
        i.reporter_id                                     AS reporter_id
    FROM {{ source('bronze_jira', 'jira_issue') }} AS i
    INNER JOIN issue_winner AS w ON i._airbyte_raw_id = w.raw_id
),

-- The same winning bronze row, carrying the payload and the moment it was
-- observed. Separate from `issues` so the JSON is read only by the one CTE
-- that needs it.
issue_json AS (
    SELECT
        COALESCE(i.source_id, '')                         AS insight_source_id,
        COALESCE(toString(i.jira_id), '')                 AS issue_id,
        COALESCE(i.custom_fields_json, '{}')              AS custom_fields_json,
        toDateTime64(i._airbyte_extracted_at, 3)          AS observed_at
    FROM {{ source('bronze_jira', 'jira_issue') }} AS i
    INNER JOIN issue_winner AS w ON i._airbyte_raw_id = w.raw_id
),

-- Every changelog item, attributed to its issue by the issue's immutable id.
-- The changelog stream stamps `id_readable` as the key at fetch time, which a
-- move between projects invalidates; `jira_id` (connector 6.1.0, filled on
-- older rows by the deploy heal) is the only identity used here. An item
-- without one names an issue the issue stream never delivered — nothing to
-- attribute it to, so it is not in the journal;
-- `assert_jira_substream_rows_without_issue_id` reports how many there are.
changelog_items AS (
    SELECT
        ci.* EXCEPT (id_readable, jira_id),
        assumeNotNull(ci.jira_id)                         AS issue_id
    FROM {{ ref('jira__changelog_items') }} AS ci
    WHERE ci.jira_id IS NOT NULL
),

-- Every changelog item that belongs to a field we model, with its delta already
-- resolved by the field's kind.
events AS (
    SELECT
        ci.insight_source_id                              AS insight_source_id,
        ci.issue_id                                       AS issue_id,
        ci.changelog_id                                   AS changelog_id,
        -- Jira's changelog id is monotonic, and it is what breaks a tie between
        -- two events of the same millisecond. It must be compared as a NUMBER:
        -- as text '101' sorts before '99', which inverts a pair of events every
        -- time the id crosses a digit-count boundary — and for an element-wise
        -- field an inverted add/remove pair changes the resulting set. The
        -- string form stays the event id, where it is an identifier, not an
        -- order.
        toUInt64OrZero(ci.changelog_id)                   AS event_ord,
        ci.created_at                                     AS event_at,
        ci.author_account_id                              AS author_id,
        ci.field_id                                       AS field_id,
        k.field_name                                      AS field_name,
        k.field_kind                                      AS field_kind,
        {{ jira_delta_action('k.field_kind', 'ci.value_from', 'ci.value_from_string',
                             'ci.value_to', 'ci.value_to_string') }}   AS delta_action,
        {{ jira_delta_sides('k.field_kind', 'ci.value_from', 'ci.value_from_string',
                            'ci.value_to', 'ci.value_to_string') }}    AS sides,
        {{ jira_delta_element('ci.value_from', 'ci.value_from_string',
                              'ci.value_to', 'ci.value_to_string') }}  AS element,
        -- Both sides spelled IDENTICALLY: the item records no change at all.
        -- Compared as the changelog wrote them, NOT after the kind resolved
        -- them: `duration` folds zero to the empty state, so a resolved
        -- comparison also swallows `null -> 0`, which is a real event (work
        -- logged against an unestimated issue, §3.5) that the journal keeps.
        COALESCE(ci.value_from, '') = COALESCE(ci.value_to, '')
            AND COALESCE(ci.value_from_string, '') = COALESCE(ci.value_to_string, '')
                                                                       AS sides_unchanged
    FROM changelog_items AS ci
    INNER JOIN kinds AS k
        ON k.insight_source_id = ci.insight_source_id
       AND k.field_id = ci.field_id
),

-- An item with nothing on either side carries no information (§6). Neither does
-- one whose two sides are spelled identically: Jira writes those as a
-- by-product of recalculating a field it did not change — a remaining estimate
-- re-stamped while an issue is closed is the common shape.
--
-- Dropping them is not merely tidiness. `initial_state` takes the `before` side
-- of the EARLIEST event, and items of one entry share (event_at, event_ord), so
-- a real change paired with a no-op left that choice to the planner: the same
-- data yielded either value from one run to the next.
--
-- Element-wise kinds are exempt, and must be: their sides carry ONE element,
-- absent on the side it is not on, so a no-op cannot arise — while an item
-- naming the same element on both sides is the RENAME of that element, whose
-- whole purpose is to carry the new display.
live_events AS (
    SELECT * FROM events
    WHERE delta_action != 'none'
      AND (field_kind IN {{ jira_element_wise_kinds() }}
           OR NOT sides_unchanged)
),

-- ── fields the catalogue does not contain ───────────────────────────────────
-- A changelog item can name a field `bronze_jira.jira_fields` has never seen,
-- and until now those items produced NOTHING: the join to the classifier is an
-- inner one, so the field and all its history disappeared silently. That is the
-- same defect class this design exists to remove, on the one input the design
-- cannot classify even in principle (§3.2).
--
-- Bronze is append-only with dedup per field, so the catalogue never forgets a
-- field it has seen once. Absence therefore means the field was deleted before
-- the first field sync — the dominant case — or created since the last one,
-- which is the dangerous one and is what the recency test separates.
--
-- The history is NOT reconstructed: the field's shape is unknowable, so there is
-- no way to read a list, a separator or an id side. One row per (issue, field)
-- carries the last value verbatim, and `event_kind` says it is best-effort so a
-- consumer counting "issues where this was ever set" can tell it from a derived
-- value.
unclassified_events AS (
    SELECT
        ci.insight_source_id                              AS insight_source_id,
        ci.issue_id                                       AS issue_id,
        ci.field_id                                       AS field_id,
        {%- set newest = "(ci.created_at, toUInt64OrZero(ci.changelog_id))" %}
        -- The item's own display name: present even when the catalogue row is not.
        argMax(ci.field_name, {{ newest }})                AS field_name,
        max(ci.created_at)                                 AS event_at,
        argMax(COALESCE(ci.value_to, ci.value_to_string, ''), {{ newest }})        AS last_id,
        argMax(COALESCE(ci.value_to_string, ci.value_to, ''), {{ newest }})        AS last_display,
        argMax(ci.author_account_id, {{ newest }})         AS author_id
    FROM changelog_items AS ci
    -- Against the WHOLE catalogue, not the modelled subset: a field that is
    -- `ignored` or `UNKNOWN` has been classified and must not land here.
    LEFT ANTI JOIN {{ ref('jira__task_field_kind') }} AS k
        ON k.insight_source_id = ci.insight_source_id
       AND k.field_id = ci.field_id
    GROUP BY ci.insight_source_id, ci.issue_id, ci.field_id
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
),

snapshot AS (
    SELECT
        s.insight_source_id                               AS insight_source_id,
        s.issue_id                                        AS issue_id,
        s.field_id                                        AS field_id,
        s.value_ids                                       AS value_ids,
        s.value_displays                                  AS value_displays
    FROM {{ ref('jira__issue_field_snapshot') }} AS s FINAL
),

-- ── fields the issue stopped carrying ───────────────────────────────────────
-- Jira emits no changelog item when a field leaves an issue's field context —
-- the project's or the issue type's configuration changed, or the field was
-- deleted from the instance. The key simply stops appearing in the issue JSON.
-- Without an event the journal's newest state stays at whatever the field last
-- held, which is a value the issue does not have; that is one class of
-- round-trip failure, and the fix is to record the withdrawal rather than to
-- exempt the pair from the check.
--
-- The cause is deliberately not classified. "Deleted from the instance" and
-- "removed from this issue's context" are the same observation from here, and
-- telling them apart would need the field catalogue's own last-seen mark to
-- agree with the issue's — two streams read at different points of one sync.
--
-- Only an ABSENT key qualifies. A key present with an empty value means the
-- field still applies to the issue and is unset (§6), which is an ordinary
-- state; if the journal disagrees with it, a clearing event is genuinely
-- missing and must surface as a failure instead of being overwritten here.
retired_candidates AS (
    SELECT
        p.insight_source_id                               AS insight_source_id,
        p.issue_id                                        AS issue_id,
        groupArray(p.field_id)                            AS field_ids
    FROM (
        SELECT DISTINCT insight_source_id, issue_id, field_id
        FROM live_events
    ) AS p
    LEFT ANTI JOIN snapshot AS s
        ON s.insight_source_id = p.insight_source_id
       AND s.issue_id = p.issue_id
       AND s.field_id = p.field_id
    GROUP BY p.insight_source_id, p.issue_id
),

-- MEMORY (§13): the candidate list is the build side and the issue JSON
-- streams past it, so a payload is read once per issue, tested with JSONHas
-- for each candidate field, and dropped. Joining one row per (issue, field)
-- against the JSON instead would carry the payload once per field.
retired_pairs AS (
    SELECT
        j.insight_source_id                               AS insight_source_id,
        j.issue_id                                        AS issue_id,
        arrayJoin(arrayFilter(f -> NOT JSONHas(j.custom_fields_json, f),
                              c.field_ids))               AS field_id,
        j.observed_at                                     AS event_at
    FROM issue_json AS j
    INNER JOIN retired_candidates AS c
        ON c.insight_source_id = j.insight_source_id
       AND c.issue_id = j.issue_id
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
        e.element.1                                       AS element_id,
        concat(e.element.1, '\x1f', e.element.2)           AS pair,
        -- The event's position in this (issue, field)'s sequence. Items of one
        -- ENTRY share (event_at, event_ord); the element id breaks that tie so
        -- the numbering is reproducible where the window's order among them was
        -- arbitrary. Items of one entry name distinct elements, so no element's
        -- own order depends on the tiebreak.
        row_number() OVER (PARTITION BY e.insight_source_id, e.issue_id, e.field_id
                           ORDER BY e.event_at, e.event_ord, e.element.1) AS seq
    FROM live_events AS e
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
),

-- ── the state the journal's own events arrive at ────────────────────────────
-- Needed to tell a field the issue silently cleared from one it never held.
-- Both families contribute: a self-describing item carries the state after it,
-- and an element-wise field's state after its last operation is the span set.
newest_from_events AS (
    SELECT
        src                                               AS insight_source_id,
        iss                                               AS issue_id,
        fid                                               AS field_id,
        argMax(ids, ord)                                  AS value_ids
    FROM (
        SELECT
            e.insight_source_id                           AS src,
            e.issue_id                                 AS iss,
            e.field_id                                    AS fid,
            (e.event_at, e.event_ord)                     AS ord,
            e.sides.3                                     AS ids
        FROM live_events AS e
        WHERE e.field_kind NOT IN {{ jira_element_wise_kinds() }}

        UNION ALL

        SELECT
            a.insight_source_id,
            a.issue_id,
            a.field_id,
            (a.event_at, a.ops_seq),
            arrayMap(x -> splitByChar('\x1f', x)[1], a.state_pairs)
        FROM element_wise_state AS a
    )
    GROUP BY src, iss, fid
),

-- ── fields the issue cleared without recording it ───────────────────────────
-- The key is STILL in the issue JSON — the field applies and is simply unset —
-- but the journal's own events end on a value. Jira does that when a value goes
-- away without an entry: a link removed from the other side of the pair, an
-- automation writing a read-only field, a list emptied by a bulk operation. §6
-- calls the missing entry what it is, and this row does not pretend otherwise:
-- it records the state observed, not an event that happened.
--
-- An ABSENT key is a different thing and belongs to `retired_pairs` — the field
-- left the issue's context, rather than the issue dropping its value.
cleared_pairs AS (
    SELECT
        n.insight_source_id                               AS insight_source_id,
        n.issue_id                                        AS issue_id,
        n.field_id                                        AS field_id,
        j.observed_at                                     AS event_at
    FROM newest_from_events AS n
    INNER JOIN issue_json AS j
        ON j.insight_source_id = n.insight_source_id
       AND j.issue_id = n.issue_id
    LEFT ANTI JOIN snapshot AS s
        ON s.insight_source_id = n.insight_source_id
       AND s.issue_id = n.issue_id
       AND s.field_id = n.field_id
    WHERE length(n.value_ids) > 0
      AND JSONHas(j.custom_fields_json, n.field_id)
),

-- ── the value of every modelled field at issue creation ─────────────────────
-- A field that changed is rolled back to before its earliest event; a field
-- that never changed keeps its snapshot value. The second case is the one the
-- current pipeline cannot produce for any field outside its hardcoded list, and
-- is why a field set at creation and never touched has no history at all.
initial_state AS (
    SELECT
        insight_source_id, issue_id, field_id, field_name, field_kind,
        value_ids, value_displays
    FROM (
        -- fields with at least one event: the earliest event's `before` side
        SELECT
            e.insight_source_id                            AS insight_source_id,
            e.issue_id                                  AS issue_id,
            e.field_id                                     AS field_id,
            argMin(e.field_name, (e.event_at, e.event_ord))  AS field_name,
            argMin(e.field_kind, (e.event_at, e.event_ord))  AS field_kind,
            argMin(e.sides.1, (e.event_at, e.event_ord))     AS value_ids,
            argMin(e.sides.2, (e.event_at, e.event_ord))     AS value_displays
        FROM live_events AS e
        WHERE e.field_kind NOT IN {{ jira_element_wise_kinds() }}
        GROUP BY e.insight_source_id, e.issue_id, e.field_id

        UNION ALL

        -- element-wise with events: the reconstructed initial set
        SELECT
            a.insight_source_id,
            a.issue_id,
            a.field_id,
            any(a.field_name)                              AS field_name,
            any(a.field_kind)                              AS field_kind,
            arrayMap(x -> splitByChar('\x1f', x)[1],
                     any(a.initial_pairs))                 AS value_ids,
            arrayMap(x -> splitByChar('\x1f', x)[2],
                     any(a.initial_pairs))                 AS value_displays
        FROM element_wise_state AS a
        GROUP BY a.insight_source_id, a.issue_id, a.field_id

        UNION ALL

        -- fields with NO event at all: the snapshot value is the initial value
        SELECT
            s.insight_source_id,
            s.issue_id,
            s.field_id,
            k.field_name                                   AS field_name,
            k.field_kind                                   AS field_kind,
            s.value_ids,
            s.value_displays
        FROM snapshot AS s
        INNER JOIN kinds AS k
            ON k.insight_source_id = s.insight_source_id
           AND k.field_id = s.field_id
        LEFT ANTI JOIN (
            SELECT DISTINCT insight_source_id, issue_id, field_id FROM live_events
        ) AS ev
            ON ev.insight_source_id = s.insight_source_id
           AND ev.issue_id = s.issue_id
           AND ev.field_id = s.field_id
    )
),

-- `_seq` is the field's 0-based index in field_id-ascending order within the
-- issue, offset by one so the creation marker keeps seq 0 (§10).
initial_seq AS (
    SELECT
        *,
        toUInt32(row_number() OVER (PARTITION BY insight_source_id, issue_id
                                    ORDER BY field_id)) AS seq
    FROM initial_state
)

-- ── row 1: the creation marker ──────────────────────────────────────────────
SELECT
    CAST({{ jira_history_key('insight_source_id', 'issue_id', "'created'", "concat('initial:', issue_id)") }} AS String) AS unique_key,
    insight_source_id,
    CAST('jira' AS String)                                AS data_source,
    issue_id,
    id_readable,
    CAST(concat('initial:', issue_id) AS String)          AS event_id,
    created_at                                            AS event_at,
    CAST('synthetic_initial' AS String)                   AS event_kind,
    toUInt32(0)                                           AS _seq,
    reporter_id                                           AS author_id,
    CAST('created' AS String)                             AS field_id,
    CAST('Created' AS String)                             AS field_name,
    CAST('single' AS String)                              AS field_cardinality,
    CAST('set' AS String)                                 AS delta_action,
    CAST([] AS Array(String))                             AS value_ids,
    CAST([] AS Array(String))                             AS value_displays,
    CAST('none' AS String)                                AS value_id_type,
    now64(3)                                              AS collected_at,
    toUnixTimestamp64Milli(now64(3))                      AS _version
FROM issues

UNION ALL

-- ── row 2: changelog rows for the self-describing kinds ─────────────────────
-- The state after the event is the item's own `to` side; nothing accumulates.
SELECT
    CAST({{ jira_history_key('e.insight_source_id', 'e.issue_id', 'e.field_id', 'e.changelog_id') }} AS String) AS unique_key,
    e.insight_source_id,
    CAST('jira' AS String)                                AS data_source,
    e.issue_id                                            AS issue_id,
    COALESCE(i.id_readable, '')                           AS id_readable,
    e.changelog_id                                        AS event_id,
    e.event_at,
    CAST('changelog' AS String)                           AS event_kind,
    toUInt32(0)                                           AS _seq,
    e.author_id,
    e.field_id,
    e.field_name,
    {{ jira_field_cardinality('e.field_kind') }}          AS field_cardinality,
    CAST('set' AS String)                                 AS delta_action,
    -- Deduplicated by id in ONE place (§5): Jira's own bracketed list can repeat
    -- an id, and per-kind dedup missed that twice.
    CAST({{ jira_distinct_arrays_by_id('e.sides.3', 'e.sides.4', 'ids') }} AS Array(String))      AS value_ids,
    CAST({{ jira_distinct_arrays_by_id('e.sides.3', 'e.sides.4', 'displays') }} AS Array(String)) AS value_displays,
    {{ jira_field_id_type('e.field_kind') }}              AS value_id_type,
    now64(3)                                              AS collected_at,
    toUnixTimestamp64Milli(now64(3))                      AS _version
FROM live_events AS e
LEFT JOIN issues AS i
    ON i.insight_source_id = e.insight_source_id
   AND i.issue_id = e.issue_id
WHERE e.field_kind NOT IN {{ jira_element_wise_kinds() }}

UNION ALL

-- ── row 3: changelog rows for the element-wise kinds, with running state ────
-- One row per changelog ENTRY, not per item. An entry that touches two elements
-- of one field arrives as two items under the same changelog id; the entry is
-- one user action, and the contract orders events by `event_id`, so two rows
-- for one id could not be told apart by a reader — and the ReplacingMergeTree
-- would keep one of them anyway. Items of one entry name distinct elements, so
-- the state after the entry is the fold that has consumed every item, in
-- whichever order the window visited them.
SELECT
    CAST({{ jira_history_key('a.insight_source_id', 'a.issue_id', 'a.field_id', 'a.changelog_id') }} AS String) AS unique_key,
    a.insight_source_id,
    CAST('jira' AS String)                                AS data_source,
    COALESCE(any(i.issue_id), '')                         AS issue_id,
    COALESCE(any(i.id_readable), '')                      AS id_readable,
    a.changelog_id                                        AS event_id,
    any(a.event_at)                                       AS event_at,
    CAST('changelog' AS String)                           AS event_kind,
    toUInt32(0)                                           AS _seq,
    any(a.author_id)                                      AS author_id,
    a.field_id,
    any(a.field_name)                                     AS field_name,
    {{ jira_field_cardinality('any(a.field_kind)') }}     AS field_cardinality,
    -- An entry that only adds or only removes keeps that verb; one that does
    -- both replaced elements, and `set` is the contract's word for that.
    if(uniqExact(a.delta_action) = 1, any(a.delta_action), 'set')    AS delta_action,
    -- state after the whole entry, as parallel arrays again
    CAST(arrayMap(x -> splitByChar('\x1f', x)[1],
                  argMax(a.state_pairs, a.ops_seq)) AS Array(String))  AS value_ids,
    CAST(arrayMap(x -> splitByChar('\x1f', x)[2],
                  argMax(a.state_pairs, a.ops_seq)) AS Array(String))  AS value_displays,
    {{ jira_field_id_type('any(a.field_kind)') }}         AS value_id_type,
    now64(3)                                              AS collected_at,
    toUnixTimestamp64Milli(now64(3))                      AS _version
FROM element_wise_state AS a
LEFT JOIN issues AS i
    ON i.insight_source_id = a.insight_source_id
   AND i.issue_id = a.issue_id
GROUP BY a.insight_source_id, a.issue_id, a.field_id, a.changelog_id


UNION ALL

-- ── row 4: one synthetic_initial per (issue, field) ─────────────────────────
SELECT
    CAST({{ jira_history_key('s.insight_source_id', 's.issue_id', 's.field_id',
                             "concat('initial:', s.issue_id)") }} AS String)  AS unique_key,
    s.insight_source_id,
    CAST('jira' AS String)                                AS data_source,
    s.issue_id                                             AS issue_id,
    COALESCE(i.id_readable, '')                           AS id_readable,
    CAST(concat('initial:', s.issue_id) AS String)             AS event_id,
    COALESCE(i.created_at, toDateTime64(0, 3))            AS event_at,
    CAST('synthetic_initial' AS String)                   AS event_kind,
    s.seq                                                 AS _seq,
    i.reporter_id                                         AS author_id,
    s.field_id,
    s.field_name,
    {{ jira_field_cardinality('s.field_kind') }}          AS field_cardinality,
    CAST('set' AS String)                                 AS delta_action,
    CAST({{ jira_distinct_arrays_by_id('s.value_ids', 's.value_displays', 'ids') }} AS Array(String))      AS value_ids,
    CAST({{ jira_distinct_arrays_by_id('s.value_ids', 's.value_displays', 'displays') }} AS Array(String)) AS value_displays,
    {{ jira_field_id_type('s.field_kind') }}              AS value_id_type,
    now64(3)                                              AS collected_at,
    toUnixTimestamp64Milli(now64(3))                      AS _version
FROM initial_seq AS s
INNER JOIN issues AS i
    ON i.insight_source_id = s.insight_source_id
   AND i.issue_id = s.issue_id

UNION ALL

-- ── row 5: the withdrawal of a field the issue no longer carries ────────────
-- Dated by the moment the absence was observed, which is the same stamp the
-- round-trip invariant uses as the issue's own freshness — so the event is
-- never newer than the state it is compared against.
SELECT
    CAST({{ jira_history_key('r.insight_source_id', 'r.issue_id', 'r.field_id',
                             "concat('retired:', r.issue_id)") }} AS String)  AS unique_key,
    r.insight_source_id,
    CAST('jira' AS String)                                AS data_source,
    r.issue_id                                             AS issue_id,
    COALESCE(i.id_readable, '')                           AS id_readable,
    CAST(concat('retired:', r.issue_id) AS String)             AS event_id,
    r.event_at,
    CAST('retired_field' AS String)                       AS event_kind,
    toUInt32(0)                                           AS _seq,
    -- Withdrawing a field is a configuration change, not an edit of the issue;
    -- the changelog carries no actor for it and Jira exposes none.
    CAST(NULL AS Nullable(String))                        AS author_id,
    r.field_id,
    k.field_name,
    {{ jira_field_cardinality('k.field_kind') }}          AS field_cardinality,
    -- Same rule the cardinality contract states for a value going away: a
    -- single field is `set` to nothing, a multi field has its elements removed.
    CAST(if({{ jira_field_cardinality('k.field_kind') }} = 'multi',
            'remove', 'set') AS String)                   AS delta_action,
    CAST([] AS Array(String))                             AS value_ids,
    CAST([] AS Array(String))                             AS value_displays,
    -- The field's own identifier kind, not 'none': `value_id_type` is asserted
    -- stable per (source, field), so a row of that field may not carry a
    -- different one just because its arrays are empty.
    {{ jira_field_id_type('k.field_kind') }}              AS value_id_type,
    now64(3)                                              AS collected_at,
    toUnixTimestamp64Milli(now64(3))                      AS _version
FROM retired_pairs AS r
INNER JOIN kinds AS k
    ON k.insight_source_id = r.insight_source_id
   AND k.field_id = r.field_id
LEFT JOIN issues AS i
    ON i.insight_source_id = r.insight_source_id
   AND i.issue_id = r.issue_id

UNION ALL

-- ── the value the issue no longer holds, with no event to explain it ────────
-- Dated by the observation, like the withdrawal row above: the moment the
-- clearing happened is not knowable, only the moment it was seen. `event_id` is
-- constant per (issue, field) so re-observing restamps the one row rather than
-- growing a new one each sync — the journal must not accumulate a row per read.
--
-- `snapshot_diff` is its own kind on purpose: a consumer counting real history
-- can exclude it, and the share of state recovered by observation rather than
-- by event stays measurable.
SELECT
    CAST({{ jira_history_key('c.insight_source_id', 'c.issue_id', 'c.field_id',
                             "concat('snapshot_diff:', c.issue_id)") }} AS String)  AS unique_key,
    c.insight_source_id,
    CAST('jira' AS String)                                AS data_source,
    c.issue_id                                             AS issue_id,
    COALESCE(i.id_readable, '')                           AS id_readable,
    CAST(concat('snapshot_diff:', c.issue_id) AS String)       AS event_id,
    c.event_at,
    CAST('snapshot_diff' AS String)                       AS event_kind,
    toUInt32(0)                                           AS _seq,
    -- Nobody is recorded as having done this: there is no entry to name an author.
    CAST(NULL AS Nullable(String))                        AS author_id,
    c.field_id,
    k.field_name,
    {{ jira_field_cardinality('k.field_kind') }}          AS field_cardinality,
    CAST(if({{ jira_field_cardinality('k.field_kind') }} = 'multi',
            'remove', 'set') AS String)                   AS delta_action,
    CAST([] AS Array(String))                             AS value_ids,
    CAST([] AS Array(String))                             AS value_displays,
    {{ jira_field_id_type('k.field_kind') }}              AS value_id_type,
    now64(3)                                              AS collected_at,
    toUnixTimestamp64Milli(now64(3))                      AS _version
FROM cleared_pairs AS c
INNER JOIN kinds AS k
    ON k.insight_source_id = c.insight_source_id
   AND k.field_id = c.field_id
LEFT JOIN issues AS i
    ON i.insight_source_id = c.insight_source_id
   AND i.issue_id = c.issue_id

UNION ALL

-- ── row 6: the last known value of a field that cannot be classified ────────
-- Values are stored as they arrived, with no list parsing: the field's shape is
-- unknowable, so any parsing rule here would be a guess of exactly the kind
-- this design replaces.
SELECT
    CAST({{ jira_history_key('u.insight_source_id', 'u.issue_id', 'u.field_id',
                             "concat('unclassified:', u.issue_id)") }} AS String)  AS unique_key,
    u.insight_source_id,
    CAST('jira' AS String)                                AS data_source,
    u.issue_id                                             AS issue_id,
    COALESCE(i.id_readable, '')                           AS id_readable,
    CAST(concat('unclassified:', u.issue_id) AS String)        AS event_id,
    u.event_at,
    CAST('unclassified_field' AS String)                  AS event_kind,
    toUInt32(0)                                           AS _seq,
    u.author_id,
    u.field_id,
    u.field_name,
    -- Unknown, so the narrower of the two: a single value is what one row of an
    -- unparsed `to` side can honestly claim to be.
    CAST('single' AS String)                              AS field_cardinality,
    CAST('set' AS String)                                 AS delta_action,
    CAST(if(u.last_id = '', [], [u.last_id]) AS Array(String))           AS value_ids,
    CAST(if(u.last_display = '', [], [u.last_display]) AS Array(String)) AS value_displays,
    -- Not `opaque_id`: nothing here establishes that the value IS an id.
    CAST('none' AS String)                                AS value_id_type,
    now64(3)                                              AS collected_at,
    toUnixTimestamp64Milli(now64(3))                      AS _version
FROM unclassified_events AS u
LEFT JOIN issues AS i
    ON i.insight_source_id = u.insight_source_id
   AND i.issue_id = u.issue_id

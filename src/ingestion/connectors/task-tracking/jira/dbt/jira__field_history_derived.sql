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
-- Five kinds of row, matching the contract the class consumers rely on (§10):
--   1. one creation marker per issue (`field_id = 'created'`, `_seq = 0`);
--   2. one `synthetic_initial` per (issue, field) holding the value at creation;
--   3. one `changelog` row per event, holding the state after that event;
--   4. one `retired_field` row per (issue, field) the issue stopped carrying;
--   5. one `unclassified_field` row per (issue, field) the catalogue lacks.
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
    SELECT unique_key, argMax(_airbyte_raw_id, _airbyte_extracted_at) AS raw_id
    FROM {{ source('bronze_jira', 'jira_issue') }}
    WHERE unique_key IS NOT NULL
    GROUP BY unique_key
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
        COALESCE(toString(i.id_readable), '')             AS id_readable,
        COALESCE(i.custom_fields_json, '{}')              AS custom_fields_json,
        toDateTime64(i._airbyte_extracted_at, 3)          AS observed_at
    FROM {{ source('bronze_jira', 'jira_issue') }} AS i
    INNER JOIN issue_winner AS w ON i._airbyte_raw_id = w.raw_id
),

-- Every changelog item that belongs to a field we model, with its delta already
-- resolved by the field's kind.
events AS (
    SELECT
        ci.insight_source_id                              AS insight_source_id,
        ci.id_readable                                    AS id_readable,
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
                              'ci.value_to', 'ci.value_to_string') }}  AS element
    FROM {{ ref('jira__changelog_items') }} AS ci
    INNER JOIN kinds AS k
        ON k.insight_source_id = ci.insight_source_id
       AND k.field_id = ci.field_id
),

-- An item with nothing on either side carries no information (§6).
live_events AS (
    SELECT * FROM events WHERE delta_action != 'none'
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
        ci.id_readable                                    AS id_readable,
        ci.field_id                                       AS field_id,
        {%- set newest = "(ci.created_at, toUInt64OrZero(ci.changelog_id))" %}
        -- The item's own display name: present even when the catalogue row is not.
        argMax(ci.field_name, {{ newest }})                AS field_name,
        max(ci.created_at)                                 AS event_at,
        argMax(COALESCE(ci.value_to, ci.value_to_string, ''), {{ newest }})        AS last_id,
        argMax(COALESCE(ci.value_to_string, ci.value_to, ''), {{ newest }})        AS last_display,
        argMax(ci.author_account_id, {{ newest }})         AS author_id
    FROM {{ ref('jira__changelog_items') }} AS ci
    -- Against the WHOLE catalogue, not the modelled subset: a field that is
    -- `ignored` or `UNKNOWN` has been classified and must not land here.
    LEFT ANTI JOIN {{ ref('jira__task_field_kind') }} AS k
        ON k.insight_source_id = ci.insight_source_id
       AND k.field_id = ci.field_id
    GROUP BY ci.insight_source_id, ci.id_readable, ci.field_id
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
        s.id_readable                                     AS id_readable,
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
        s.id_readable                                     AS id_readable,
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
        p.id_readable                                     AS id_readable,
        groupArray(p.field_id)                            AS field_ids
    FROM (
        SELECT DISTINCT insight_source_id, id_readable, field_id
        FROM live_events
    ) AS p
    LEFT ANTI JOIN snapshot AS s
        ON s.insight_source_id = p.insight_source_id
       AND s.id_readable = p.id_readable
       AND s.field_id = p.field_id
    GROUP BY p.insight_source_id, p.id_readable
),

-- MEMORY (§13): the candidate list is the build side and the issue JSON
-- streams past it, so a payload is read once per issue, tested with JSONHas
-- for each candidate field, and dropped. Joining one row per (issue, field)
-- against the JSON instead would carry the payload once per field.
retired_pairs AS (
    SELECT
        j.insight_source_id                               AS insight_source_id,
        j.id_readable                                     AS id_readable,
        arrayJoin(arrayFilter(f -> NOT JSONHas(j.custom_fields_json, f),
                              c.field_ids))               AS field_id,
        j.observed_at                                     AS event_at
    FROM issue_json AS j
    INNER JOIN retired_candidates AS c
        ON c.insight_source_id = j.insight_source_id
       AND c.id_readable = j.id_readable
),

-- ── the element-wise kinds, whose state accumulates ────────────────────────
-- Elements are carried as one string per element so ids and displays cannot
-- drift apart; they are split back into the parallel arrays at the end.
--
-- The field's own operations are collected in order, once up to each row and
-- once in full. The initial state is the current value with the full list
-- UNDONE, and the state after event k is that initial with the list up to k
-- APPLIED. See `jira_apply_ops` for why this is a fold rather than the set
-- arithmetic it replaced.
element_wise_events AS (
    SELECT
        e.*,
        concat(e.element.1, '\x1f', e.element.2)          AS pair,
        groupArray((e.delta_action, concat(e.element.1, '\x1f', e.element.2)))
            OVER (PARTITION BY e.insight_source_id, e.id_readable, e.field_id
                  ORDER BY e.event_at, e.event_ord
                  ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)   AS ops_upto,
        -- The whole list, still ordered: a window without ORDER BY collects in
        -- an arbitrary order, and undoing operations out of order is wrong for
        -- exactly the cycles this fold exists to handle.
        groupArray((e.delta_action, concat(e.element.1, '\x1f', e.element.2)))
            OVER (PARTITION BY e.insight_source_id, e.id_readable, e.field_id
                  ORDER BY e.event_at, e.event_ord
                  ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING) AS ops_all
    FROM live_events AS e
    WHERE e.field_kind IN {{ jira_element_wise_kinds() }}
),

element_wise_state AS (
    SELECT
        a.*,
        {{ jira_undo_ops("a.ops_all", jira_distinct_pairs_by_id("arrayMap(j -> concat(s.value_ids[j], '\x1f', s.value_displays[j]), range(1, length(s.value_ids) + 1))")) }}
                                                          AS initial_pairs,
        {{ jira_apply_ops("a.ops_upto", jira_undo_ops("a.ops_all", jira_distinct_pairs_by_id("arrayMap(j -> concat(s.value_ids[j], '\x1f', s.value_displays[j]), range(1, length(s.value_ids) + 1))"))) }}
                                                          AS state_pairs
    FROM element_wise_events AS a
    LEFT JOIN snapshot_element_wise AS s
        ON s.insight_source_id = a.insight_source_id
       AND s.id_readable = a.id_readable
       AND s.field_id = a.field_id
),

-- ── the value of every modelled field at issue creation ─────────────────────
-- A field that changed is rolled back to before its earliest event; a field
-- that never changed keeps its snapshot value. The second case is the one the
-- current pipeline cannot produce for any field outside its hardcoded list, and
-- is why a field set at creation and never touched has no history at all.
initial_state AS (
    SELECT
        insight_source_id, id_readable, field_id, field_name, field_kind,
        value_ids, value_displays
    FROM (
        -- fields with at least one event: the earliest event's `before` side
        SELECT
            e.insight_source_id                            AS insight_source_id,
            e.id_readable                                  AS id_readable,
            e.field_id                                     AS field_id,
            argMin(e.field_name, (e.event_at, e.event_ord))  AS field_name,
            argMin(e.field_kind, (e.event_at, e.event_ord))  AS field_kind,
            argMin(e.sides.1, (e.event_at, e.event_ord))     AS value_ids,
            argMin(e.sides.2, (e.event_at, e.event_ord))     AS value_displays
        FROM live_events AS e
        WHERE e.field_kind NOT IN {{ jira_element_wise_kinds() }}
        GROUP BY e.insight_source_id, e.id_readable, e.field_id

        UNION ALL

        -- element-wise with events: the reconstructed initial set
        SELECT
            a.insight_source_id,
            a.id_readable,
            a.field_id,
            any(a.field_name)                              AS field_name,
            any(a.field_kind)                              AS field_kind,
            arrayMap(x -> splitByChar('\x1f', x)[1],
                     any(a.initial_pairs))                 AS value_ids,
            arrayMap(x -> splitByChar('\x1f', x)[2],
                     any(a.initial_pairs))                 AS value_displays
        FROM element_wise_state AS a
        GROUP BY a.insight_source_id, a.id_readable, a.field_id

        UNION ALL

        -- fields with NO event at all: the snapshot value is the initial value
        SELECT
            s.insight_source_id,
            s.id_readable,
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
            SELECT DISTINCT insight_source_id, id_readable, field_id FROM live_events
        ) AS ev
            ON ev.insight_source_id = s.insight_source_id
           AND ev.id_readable = s.id_readable
           AND ev.field_id = s.field_id
    )
),

-- `_seq` is the field's 0-based index in field_id-ascending order within the
-- issue, offset by one so the creation marker keeps seq 0 (§10).
initial_seq AS (
    SELECT
        *,
        toUInt32(row_number() OVER (PARTITION BY insight_source_id, id_readable
                                    ORDER BY field_id)) AS seq
    FROM initial_state
)

-- ── row 1: the creation marker ──────────────────────────────────────────────
SELECT
    CAST(concat(insight_source_id, '-jira-', id_readable, '-created-initial:', issue_id) AS String) AS unique_key,
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
    CAST(concat(e.insight_source_id, '-jira-', e.id_readable, '-', e.field_id, '-', e.changelog_id) AS String) AS unique_key,
    e.insight_source_id,
    CAST('jira' AS String)                                AS data_source,
    COALESCE(i.issue_id, '')                              AS issue_id,
    e.id_readable,
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
   AND i.id_readable = e.id_readable
WHERE e.field_kind NOT IN {{ jira_element_wise_kinds() }}

UNION ALL

-- ── row 3: changelog rows for the element-wise kinds, with running state ────
SELECT
    CAST(concat(a.insight_source_id, '-jira-', a.id_readable, '-', a.field_id, '-', a.changelog_id) AS String) AS unique_key,
    a.insight_source_id,
    CAST('jira' AS String)                                AS data_source,
    COALESCE(i.issue_id, '')                              AS issue_id,
    a.id_readable,
    a.changelog_id                                        AS event_id,
    a.event_at,
    CAST('changelog' AS String)                           AS event_kind,
    toUInt32(0)                                           AS _seq,
    a.author_id,
    a.field_id,
    a.field_name,
    {{ jira_field_cardinality('a.field_kind') }}          AS field_cardinality,
    a.delta_action,
    -- state after this event, as parallel arrays again
    CAST(arrayMap(x -> splitByChar('\x1f', x)[1], a.state_pairs) AS Array(String)) AS value_ids,
    CAST(arrayMap(x -> splitByChar('\x1f', x)[2], a.state_pairs) AS Array(String)) AS value_displays,
    {{ jira_field_id_type('a.field_kind') }}              AS value_id_type,
    now64(3)                                              AS collected_at,
    toUnixTimestamp64Milli(now64(3))                      AS _version
FROM element_wise_state AS a
LEFT JOIN issues AS i
    ON i.insight_source_id = a.insight_source_id
   AND i.id_readable = a.id_readable


UNION ALL

-- ── row 4: one synthetic_initial per (issue, field) ─────────────────────────
SELECT
    CAST(concat(s.insight_source_id, '-jira-', s.id_readable, '-', s.field_id,
                '-initial:', COALESCE(i.issue_id, '')) AS String)  AS unique_key,
    s.insight_source_id,
    CAST('jira' AS String)                                AS data_source,
    COALESCE(i.issue_id, '')                              AS issue_id,
    s.id_readable,
    CAST(concat('initial:', COALESCE(i.issue_id, '')) AS String) AS event_id,
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
   AND i.id_readable = s.id_readable

UNION ALL

-- ── row 5: the withdrawal of a field the issue no longer carries ────────────
-- Dated by the moment the absence was observed, which is the same stamp the
-- round-trip invariant uses as the issue's own freshness — so the event is
-- never newer than the state it is compared against.
SELECT
    CAST(concat(r.insight_source_id, '-jira-', r.id_readable, '-', r.field_id,
                '-retired:', COALESCE(i.issue_id, '')) AS String)  AS unique_key,
    r.insight_source_id,
    CAST('jira' AS String)                                AS data_source,
    COALESCE(i.issue_id, '')                              AS issue_id,
    r.id_readable,
    CAST(concat('retired:', COALESCE(i.issue_id, '')) AS String) AS event_id,
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
   AND i.id_readable = r.id_readable

UNION ALL

-- ── row 6: the last known value of a field that cannot be classified ────────
-- Values are stored as they arrived, with no list parsing: the field's shape is
-- unknowable, so any parsing rule here would be a guess of exactly the kind
-- this design replaces.
SELECT
    CAST(concat(u.insight_source_id, '-jira-', u.id_readable, '-', u.field_id,
                '-unclassified:', COALESCE(i.issue_id, '')) AS String)  AS unique_key,
    u.insight_source_id,
    CAST('jira' AS String)                                AS data_source,
    COALESCE(i.issue_id, '')                              AS issue_id,
    u.id_readable,
    CAST(concat('unclassified:', COALESCE(i.issue_id, '')) AS String) AS event_id,
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
   AND i.id_readable = u.id_readable

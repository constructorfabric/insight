-- depends_on: {{ ref('jira__bronze_promoted') }}
-- depends_on: {{ ref('jira__task_field_kind') }}
-- depends_on: {{ ref('jira__issue_field_snapshot') }}
-- depends_on: {{ ref('jira__changelog_items') }}
-- depends_on: {{ ref('jira__changelog_entry_ranks') }}
{{ config(
    materialized='incremental',
    incremental_strategy='delete+insert',
    unique_key=['insight_source_id', 'issue_id'],
    alias='jira__field_history_derived',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    query_settings={
        'max_bytes_before_external_group_by': 2000000000,
        'max_bytes_before_external_sort': 2000000000,
    },
    pre_hook=[
        "{{ jira_journal_drop_rows_bronze_cannot_account_for() }}",
        "{{ jira_journal_prepare_scope() }}",
    ],
    post_hook=[
        "{{ jira_journal_derive_remaining_batches() }}",
        "{{ jira_journal_drop_stages() }}",
        "{{ jira_journal_record_catalogue() }}",
    ],
    tags=['staging', 'jira', 'silver:class_task_field_history']
) }}

{%- set catalogue = jira_journal_catalogue_state() -%}

-- The per-(issue x field x event) journal, derived in dbt. The Jira producer of
-- `silver.class_task_field_history`, joined there by the availability and
-- lifecycle arms and by the GitHub arm. See
-- `connectors/task-tracking/jira/specs/FIELD-HISTORY-IN-DBT.md`.
--
-- Recomputed per ISSUE (§7). The element-wise kinds fold state across an
-- issue's whole event sequence, so no event can be appended on its own: an
-- issue that received anything since the last run has its complete journal
-- derived again and every row it held replaced. Issues bronze did not touch
-- are not read. A field catalogue that differs from the one the last build
-- used rebuilds the whole table, because `retired_field`,
-- `unclassified_field` and `synthetic_initial` rows depend on it
-- (`jira_journal_catalogue_state`). A scope larger than one batch of issues is
-- derived one batch per statement, this one and the post-hook's replays of it
-- (`jira_journal_prepare_scope`), so a rebuild needs the memory of a batch.
-- The relations read most are the batch's stages, written by those hooks
-- (`jira_journal_derive_stages`): the arms read them instead of re-deriving.
--
-- INVARIANT: `delete+insert` alongside ReplacingMergeTree is deliberate, not
-- two dedup mechanisms stacked. The engine collapses a key re-emitted with a
-- newer version; only the delete removes a row that should no longer exist at
-- all — an item its changelog entry no longer carries, a `retired_field` whose
-- field came back. The class this feeds is keyed and materialized the same way.
--
-- Six kinds of row, matching the contract the class consumers rely on (§10):
--   1. one creation marker per issue (`field_id = 'created'`, `_seq = 0`);
--   2. one `synthetic_initial` per (issue, field) holding the value at creation;
--   3. one `changelog` row per event, holding the state after that event;
--   4. one `retired_field` row per (issue, field) the issue stopped carrying;
--   5. one `unclassified_field` row per (issue, field) the catalogue lacks;
--   6. one `snapshot_diff` per (issue, field) whose events end on a value the
--      issue does not hold — the state as OBSERVED, which is not the same
--      claim as an event.
--
-- Only the element-wise kinds accumulate state across events; every other
-- kind's item carries both sides in full, so its rows are computed from the
-- item alone (§2.1). That is why there is no general fold here.

WITH kinds AS (
    SELECT * FROM {{ jira_journal_stage('kinds') }}
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
      AND {{ jira_journal_in_batch("COALESCE(source_id, '')", "COALESCE(toString(jira_id), '')") }}
    GROUP BY source_id, jira_id
),

-- The scope predicate is repeated on every read of the issue table rather than
-- left to the join on `raw_id`: it is what lets the scan skip the JSON of the
-- issues this run does not recompute.
issues AS (
    SELECT
        COALESCE(i.source_id, '')                         AS insight_source_id,
        COALESCE(toString(i.jira_id), '')                 AS issue_id,
        COALESCE(toString(i.id_readable), '')             AS id_readable,
        COALESCE(parseDateTime64BestEffortOrNull(i.created, 3),
                 toDateTime64(0, 3))                      AS created_at,
        i.reporter_id                                     AS reporter_id,
        toDateTime64(i._airbyte_extracted_at, 3)          AS observed_at
    FROM {{ source('bronze_jira', 'jira_issue') }} AS i
    INNER JOIN issue_winner AS w ON i._airbyte_raw_id = w.raw_id
    WHERE {{ jira_journal_in_batch("COALESCE(i.source_id, '')", "COALESCE(toString(i.jira_id), '')") }}
),

-- The same winning bronze row, carrying the payload and the moment it was
-- observed. Separate from `issues` so the JSON is read only by the one CTE
-- that needs it.
issue_json AS (
    SELECT
        COALESCE(i.source_id, '')                         AS insight_source_id,
        COALESCE(toString(i.jira_id), '')                 AS issue_id,
        COALESCE(i.custom_fields_json, '{}')              AS custom_fields_json,
        toDateTime64(i._airbyte_extracted_at, 3)          AS observed_at,
        parseDateTime64BestEffortOrNull(
            JSONExtractString(COALESCE(i.custom_fields_json, '{}'), 'resolutiondate'), 3)
                                                          AS resolved_at
    FROM {{ source('bronze_jira', 'jira_issue') }} AS i
    INNER JOIN issue_winner AS w ON i._airbyte_raw_id = w.raw_id
    WHERE {{ jira_journal_in_batch("COALESCE(i.source_id, '')", "COALESCE(toString(i.jira_id), '')") }}
),

changelog_items AS (
{{- jira_journal_changelog_items_sql() -}}
),

-- The newest moment bronze received anything about an issue: its own row or
-- any of its changelog entries. Every journal row of the issue carries it as
-- `_version`, so a rebuild over unchanged bronze reproduces the same versions
-- and the class's incremental filter leaves the issue alone; an issue that
-- received anything has all its rows re-emitted under the new version. Every
-- row's issue is in one of the two inputs, so no row is left without one.
--
-- `origin_at` is the earlier of the issue's creation and its first changelog
-- entry: the `synthetic_initial` rows order there, since an imported history
-- can date an entry before the creation.
issue_freshness AS (
    SELECT
        insight_source_id,
        issue_id,
        max(observed_at)                                  AS fresh_at,
        min(origin_at)                                    AS origin_at
    FROM (
        SELECT insight_source_id, issue_id, observed_at, created_at AS origin_at FROM issues
        UNION ALL
        SELECT insight_source_id, issue_id, extracted_at AS observed_at, created_at AS origin_at FROM changelog_items
    )
    GROUP BY insight_source_id, issue_id
),

-- `jira_journal_ranked_events_sql`: the modelled, live items, ranked.
live_events AS (
    SELECT * FROM {{ jira_journal_stage('ranked_events') }}
),

-- `jira_journal_ordered_events_sql`: one event per entry and self-describing field.
ordered_events AS (
    SELECT * FROM {{ jira_journal_stage('ordered_events') }}
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

snapshot AS (
    SELECT
        s.insight_source_id                               AS insight_source_id,
        s.issue_id                                        AS issue_id,
        s.field_id                                        AS field_id,
        s.value_ids                                       AS value_ids,
        s.value_displays                                  AS value_displays
    FROM {{ ref('jira__issue_field_snapshot') }} AS s FINAL
    WHERE {{ jira_journal_in_batch('s.insight_source_id', 's.issue_id') }}
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

-- `jira_journal_element_wise_state_sql`: one row per element-wise operation,
-- with the state it produced and the field's initial set.
element_wise_state AS (
    SELECT * FROM {{ jira_journal_stage('element_wise_state') }}
),


-- ── the state the journal's own events arrive at ────────────────────────────
-- Needed to tell a field the issue changed without recording it from one whose
-- events reach its value. Both families contribute: a self-describing item
-- carries the state after it, and an element-wise field's state after its last
-- operation is the span set. Digests, not arrays: this is the build side of the
-- join below, and a hash table holding every pair's arrays is the shape §13
-- exists to avoid. The digests are taken per row, inside the union, so the
-- aggregation state below holds three scalars per (issue, field), never arrays.
newest_from_events AS (
    SELECT
        src                                                AS insight_source_id,
        iss                                                AS issue_id,
        fid                                                AS field_id,
        argMax((holds, ids_d, displays_d), ord).1          AS holds_value,
        argMax((holds, ids_d, displays_d), ord).2          AS ids_digest,
        argMax((holds, ids_d, displays_d), ord).3          AS displays_digest,
        max(ord).1                                         AS last_event_at
    FROM (
        SELECT
            e.insight_source_id                            AS src,
            e.issue_id                                     AS iss,
            e.field_id                                     AS fid,
            (e.event_at, e.entry_rank, e.event_ord)         AS ord,
            length(e.sides.3) > 0                          AS holds,
            {{ jira_value_multiset_digest(jira_distinct_arrays_by_id('e.sides.3', 'e.sides.4', 'ids')) }}       AS ids_d,
            {{ jira_value_multiset_digest(jira_distinct_arrays_by_id('e.sides.3', 'e.sides.4', 'displays')) }}  AS displays_d
        FROM ordered_events AS e

        UNION ALL

        SELECT
            a.insight_source_id,
            a.issue_id,
            a.field_id,
            (a.event_at, toUInt32(0), toUInt64(a.ops_seq)),
            length(a.state_pairs) > 0,
            {{ jira_value_multiset_digest("arrayMap(x -> splitByChar('\\x1f', x)[1], a.state_pairs)") }},
            {{ jira_value_multiset_digest("arrayMap(x -> splitByChar('\\x1f', x)[2], a.state_pairs)") }}
        FROM element_wise_state AS a
    )
    GROUP BY src, iss, fid
),

-- ── values the issue holds that its events never reach ──────────────────────
-- Jira changes a value without an entry more often than it should: a link
-- removed from the other side of the pair, an automation writing a read-only
-- field, a list emptied by a bulk operation, history imported from another
-- tracker with entries missing. §6 calls the missing entry what it is, and the
-- row this feeds does not pretend otherwise: it records the state observed,
-- not an event that happened.
--
-- Two disagreements qualify. `cleared`: the snapshot holds nothing — no row, or
-- a row whose value normalizes to empty (a `duration` of 0, §3.5) — so the key
-- must still be present in the issue JSON — an ABSENT key belongs to
-- `retired_pairs`, the field having left the issue's context. `differs`: the
-- snapshot holds another value, and ids AND displays both disagree. Ids alone
-- are a migrated instance whose options were recreated under new ids (§3.4),
-- displays alone a rename; neither is a change of value.
--
-- The snapshot streams on the left, carrying the arrays a `differs` row needs;
-- only the per-pair digests are hashed.
snapshot_disagreements AS (
    SELECT
        n.insight_source_id                               AS insight_source_id,
        n.issue_id                                        AS issue_id,
        n.field_id                                        AS field_id,
        if(ifNull(s.in_snapshot, 0) = 1 AND length(s.value_ids) > 0,
           'differs', 'cleared')                          AS disagreement,
        n.last_event_at                                   AS last_event_at,
        s.value_ids                                       AS snapshot_ids,
        s.value_displays                                  AS snapshot_displays
    FROM (
        SELECT
            insight_source_id,
            issue_id,
            field_id,
            value_ids,
            value_displays,
            toUInt8(1)                                    AS in_snapshot
        FROM snapshot
    ) AS s
    RIGHT JOIN newest_from_events AS n
        ON s.insight_source_id = n.insight_source_id
       AND s.issue_id = n.issue_id
       AND s.field_id = n.field_id
    WHERE ((ifNull(s.in_snapshot, 0) = 0 OR length(s.value_ids) = 0) AND n.holds_value)
       OR (ifNull(s.in_snapshot, 0) = 1
           AND length(s.value_ids) > 0
           AND {{ jira_value_multiset_digest(jira_distinct_arrays_by_id('s.value_ids', 's.value_displays', 'ids')) }} != n.ids_digest
           AND {{ jira_value_multiset_digest(jira_distinct_arrays_by_id('s.value_ids', 's.value_displays', 'displays')) }} != n.displays_digest)
),

-- Every key the issue JSON carries, one row each, streamed out of the JSON
-- column. MEMORY (§13): this is the LEFT side of the join below on purpose.
-- The JSON column is gigabytes wide, and a join that puts `issue_json` on the
-- right builds a hash table holding every issue's payload — the shape that
-- can exceed a server's memory budget on its own. Streaming the keys and
-- hashing the small pair set instead keeps the join to the pair set's size.
present_keys AS (
    SELECT
        j.insight_source_id                               AS insight_source_id,
        j.issue_id                                        AS issue_id,
        k                                                 AS field_id,
        j.observed_at                                     AS observed_at,
        j.resolved_at                                     AS resolved_at
    FROM issue_json AS j
    ARRAY JOIN JSONExtractKeys(j.custom_fields_json) AS k
),

-- A pair whose events are newer than the issue row is excluded, `cleared` as
-- much as `differs`: the issue stream and its changelog substream are read at
-- different moments of one sync, and a snapshot that has not caught up is not a
-- disagreement (§7).
--
-- The date of a `differs` row must be stable across syncs, or a closure it
-- records slides forward every time the issue is recomputed. A status the issue
-- now holds as done is dated by the resolution, when Jira resolved it after
-- the last recorded event; anything else sorts one millisecond after that
-- event, the earliest moment the missing change can have happened. A `cleared`
-- row keeps the observation stamp.
snapshot_diff_pairs AS (
    SELECT
        m.insight_source_id                               AS insight_source_id,
        m.issue_id                                        AS issue_id,
        m.field_id                                        AS field_id,
        m.snapshot_ids                                    AS snapshot_ids,
        m.snapshot_displays                               AS snapshot_displays,
        multiIf(
            m.disagreement = 'cleared',
                p.observed_at,
            m.field_id = 'status'
                AND (m.insight_source_id, m.snapshot_ids[1]) IN (
                        SELECT insight_source_id, status_id
                        FROM {{ ref('jira__task_statuses') }}
                        WHERE status_category = 'done'
                    )
                AND ifNull(p.resolved_at > m.last_event_at, 0),
                assumeNotNull(p.resolved_at),
            m.last_event_at + toIntervalMillisecond(1)
        )                                                 AS event_at
    FROM present_keys AS p
    INNER JOIN snapshot_disagreements AS m
        ON m.insight_source_id = p.insight_source_id
       AND m.issue_id = p.issue_id
       AND m.field_id = p.field_id
    WHERE m.last_event_at <= p.observed_at

    UNION ALL

    -- A done status with no status entry at all (§6.1). Its only row would be
    -- the `synthetic_initial` the snapshot seeds, so the issue would read as
    -- closed the moment it was created; the resolution dates the close instead.
    SELECT
        j.insight_source_id,
        j.issue_id,
        d.field_id,
        d.value_ids,
        d.value_displays,
        assumeNotNull(j.resolved_at)
    FROM issue_json AS j
    INNER JOIN (
        SELECT
            s.insight_source_id,
            s.issue_id,
            s.field_id,
            s.value_ids,
            s.value_displays
        FROM snapshot AS s
        LEFT ANTI JOIN (
            SELECT DISTINCT insight_source_id, issue_id
            FROM live_events
            WHERE field_id = 'status'
        ) AS ev
            ON ev.insight_source_id = s.insight_source_id
           AND ev.issue_id = s.issue_id
        WHERE s.field_id = 'status'
          AND (s.insight_source_id, s.value_ids[1]) IN (
                  SELECT insight_source_id, status_id
                  FROM {{ ref('jira__task_statuses') }}
                  WHERE status_category = 'done'
              )
    ) AS d
        ON d.insight_source_id = j.insight_source_id
       AND d.issue_id = j.issue_id
    INNER JOIN issues AS i
        ON i.insight_source_id = j.insight_source_id
       AND i.issue_id = j.issue_id
    WHERE ifNull(j.resolved_at > i.created_at, 0)
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
            e.issue_id                                     AS issue_id,
            e.field_id                                     AS field_id,
            argMin(e.field_name, (e.event_at, e.entry_rank, e.event_ord))  AS field_name,
            argMin(e.field_kind, (e.event_at, e.entry_rank, e.event_ord))  AS field_kind,
            argMin(e.sides.1, (e.event_at, e.entry_rank, e.event_ord))     AS value_ids,
            argMin(e.sides.2, (e.event_at, e.entry_rank, e.event_ord))     AS value_displays
        FROM ordered_events AS e
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
),

-- The six kinds of row, each arm typed loosely; the projection below fixes the
-- class types once.
journal AS (

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
    toUInt32(0)                                           AS order_rank,
    reporter_id                                           AS author_id,
    CAST('created' AS String)                             AS field_id,
    CAST('Created' AS String)                             AS field_name,
    CAST('single' AS String)                              AS field_cardinality,
    CAST('set' AS String)                                 AS delta_action,
    CAST([] AS Array(String))                             AS value_ids,
    CAST([] AS Array(String))                             AS value_displays,
    CAST('none' AS String)                                AS value_id_type,
    now64(3)                                              AS collected_at
FROM issues

UNION ALL

-- ── row 2: changelog rows for the self-describing kinds ─────────────────────
-- The state after the event is the entry's `to` side; nothing accumulates.
SELECT
    CAST({{ jira_history_key('e.insight_source_id', 'e.issue_id', 'e.field_id', 'e.changelog_id') }} AS String) AS unique_key,
    e.insight_source_id,
    CAST('jira' AS String)                                AS data_source,
    e.issue_id                                            AS issue_id,
    COALESCE(i.id_readable, '')                           AS id_readable,
    e.changelog_id                                        AS event_id,
    e.event_at,
    CAST('changelog' AS String)                           AS event_kind,
    e.entry_rank                                          AS _seq,
    e.entry_rank                                          AS order_rank,
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
    now64(3)                                              AS collected_at
FROM ordered_events AS e
LEFT JOIN issues AS i
    ON i.insight_source_id = e.insight_source_id
   AND i.issue_id = e.issue_id

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
    any(a.entry_rank)                                     AS _seq,
    any(a.entry_rank)                                     AS order_rank,
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
    now64(3)                                              AS collected_at
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
    s.seq                                                 AS order_rank,
    i.reporter_id                                         AS author_id,
    s.field_id,
    s.field_name,
    {{ jira_field_cardinality('s.field_kind') }}          AS field_cardinality,
    CAST('set' AS String)                                 AS delta_action,
    CAST({{ jira_distinct_arrays_by_id('s.value_ids', 's.value_displays', 'ids') }} AS Array(String))      AS value_ids,
    CAST({{ jira_distinct_arrays_by_id('s.value_ids', 's.value_displays', 'displays') }} AS Array(String)) AS value_displays,
    {{ jira_field_id_type('s.field_kind') }}              AS value_id_type,
    now64(3)                                              AS collected_at
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
    toUInt32(0)                                           AS order_rank,
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
    now64(3)                                              AS collected_at
FROM retired_pairs AS r
INNER JOIN kinds AS k
    ON k.insight_source_id = r.insight_source_id
   AND k.field_id = r.field_id
LEFT JOIN issues AS i
    ON i.insight_source_id = r.insight_source_id
   AND i.issue_id = r.issue_id

UNION ALL

-- ── the value the issue holds, with no event to explain it ──────────────────
-- `event_id` is constant per (issue, field) so re-observing replaces the one row
-- rather than growing a new one each sync — the journal must not accumulate a
-- row per read.
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
    toUInt32(0)                                           AS order_rank,
    -- Nobody is recorded as having done this: there is no entry to name an author.
    CAST(NULL AS Nullable(String))                        AS author_id,
    c.field_id,
    k.field_name,
    {{ jira_field_cardinality('k.field_kind') }}          AS field_cardinality,
    -- Clearing a multi field removes its elements; any other disagreement
    -- replaces the state, which the contract calls `set`.
    CAST(if(length(c.snapshot_ids) = 0
                AND {{ jira_field_cardinality('k.field_kind') }} = 'multi',
            'remove', 'set') AS String)                   AS delta_action,
    CAST({{ jira_distinct_arrays_by_id('c.snapshot_ids', 'c.snapshot_displays', 'ids') }} AS Array(String))      AS value_ids,
    CAST({{ jira_distinct_arrays_by_id('c.snapshot_ids', 'c.snapshot_displays', 'displays') }} AS Array(String)) AS value_displays,
    {{ jira_field_id_type('k.field_kind') }}              AS value_id_type,
    now64(3)                                              AS collected_at
FROM snapshot_diff_pairs AS c
INNER JOIN kinds AS k
    ON k.insight_source_id = c.insight_source_id
   AND k.field_id = c.field_id
LEFT JOIN issues AS i
    ON i.insight_source_id = c.insight_source_id
   AND i.issue_id = c.issue_id
-- The issue JSON holds an ADF document and the changelog Jira's rendering of it,
-- so the two content addresses of a `long_text` value never agree (§8).
WHERE length(c.snapshot_ids) = 0
   OR k.field_kind != 'long_text'

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
    toUInt32(0)                                           AS order_rank,
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
    now64(3)                                              AS collected_at
FROM unclassified_events AS u
LEFT JOIN issues AS i
    ON i.insight_source_id = u.insight_source_id
   AND i.issue_id = u.issue_id
)

-- The class contract's column order and types. The discriminators are
-- `LowCardinality(String)`, not enums: every source contributes its own arm to
-- the class, and an enum would make each of them name the values of all the
-- others.
--
-- `_version` is the issue's input freshness (`issue_freshness`), not the build
-- time. A build-time stamp would make every rebuild look new to
-- `class_task_field_history`, whose incremental filter admits rows above its
-- newest version, and the class would rewrite the whole Jira journal.
--
-- It is floored at the catalogue's extraction stamp as of the last rebuild, so
-- rows a rebuild changed for issues whose own inputs never moved still carry
-- something newer than the run before it delivered. Between rebuilds the floor
-- is constant and moves nothing. A change to the models alone still needs a
-- full refresh, which a major descriptor bump dispatches.
SELECT
    j.unique_key,
    j.insight_source_id,
    j.data_source,
    j.issue_id,
    j.id_readable,
    j.event_id,
    j.event_at,
    CAST(j.event_kind AS LowCardinality(String))        AS event_kind,
    j._seq,
    {{ task_event_order("if(j.event_kind = 'synthetic_initial', least(j.event_at, f.origin_at), j.event_at)",
                        task_event_band('j.event_kind'), 'j.order_rank') }}  AS event_order,
    j.author_id,
    j.field_id,
    j.field_name,
    CAST(j.field_cardinality AS LowCardinality(String)) AS field_cardinality,
    CAST(j.delta_action AS LowCardinality(String))      AS delta_action,
    j.value_ids,
    j.value_displays,
    CAST(j.value_id_type AS LowCardinality(String))     AS value_id_type,
    j.collected_at,
    toUInt64(greatest(toUnixTimestamp64Milli(f.fresh_at),
                      toInt64({{ catalogue.epoch_ms }})))  AS _version
FROM journal AS j
INNER JOIN issue_freshness AS f
    ON f.insight_source_id = j.insight_source_id
   AND f.issue_id = j.issue_id

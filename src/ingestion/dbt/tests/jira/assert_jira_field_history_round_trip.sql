{{ config(
    severity='warn',
    tags=['connector_quality', 'jira'],
    store_failures=true,
    meta={
        'title': 'Task field history reconciles with the issue snapshot',
        'domain': 'task-tracking',
        'category': 'reconciliation',
        'tier': 'error',
        'remediation': 'Replaying a field\'s history forward must land on the value the issue holds now. A finding is one of three things, and the fix differs: (1) the pipeline mis-parses the field — check the kind rules in FIELD-HISTORY-IN-DBT.md §3.3 against the field\'s `schema_type` / `schema_custom`, and note that a separator or bracketed-id rule written for one shape silently corrupts another; (2) the source and its own changelog disagree — automation and read-only fields change a value without recording an event, and a migrated instance can carry values that differ from the events that set them (a date off by a day, an instant off by a fixed offset, both sides already normalized). No transformation recovers an event the source never wrote, so the journal legitimately lags; before suspecting the date handling here, compare the RAW `value_to` of the newest changelog item with the raw issue JSON — if those two already differ, the pipeline is faithful; (3) the value\'s identifier is not stable across the two sides, which `assert_jira_field_id_spaces_intersect` reports separately and which happens when an instance is migrated and its option values are recreated under new ids. Compare the DISPLAY sides of a sample to tell (3) from (1): equal displays with different ids is (3).'
    }
) }}

-- The oracle this pipeline has never had.
--
-- SEVERITY: warn. This test reports a condition of the SOURCE, not a fault of
-- the pipeline, and the condition cannot be repaired from here. A failing gate
-- that can never be made to pass stops being a signal and starts being noise
-- someone routes around — so it warns in the nightly run and the reasons are
-- enumerated in the spec. The transformation lane runs it at ERROR strictness
-- over controlled inputs, where a real regression WOULD be a pipeline fault.
--
-- Replaying a field's history forward must land on the value the issue actually
-- holds. Concretely: for every (issue, field), the state carried by the newest
-- row of the derived journal must equal the current value in the field
-- snapshot.
--
-- This is what validates the separator and shape rules of
-- FIELD-HISTORY-IN-DBT.md §3.3 against a deployment's own data, on every run,
-- rather than against the catalogue this repository was written from. Both
-- defects the design replaces would have failed it: a labels-type field whose
-- events were discarded never reaches its current value, and a bracketed-id
-- field whose id is parsed as the literal `[a, b]` lands on a value the
-- snapshot does not contain.
--
-- Deliberately NOT compared:
--
--   * pairs whose newest event is newer than the issue's own bronze row. The
--     issue stream and the history stream are read at different points inside
--     one sync, so an event the snapshot has not caught up with is expected and
--     self-heals on the next run (§7).
--   * `long_text`: the issue JSON holds an ADF document and the changelog Jira's
--     rendering of it, so the two content addresses cannot agree (§8).
--   * element ORDER within a multi-value field. Jira does not promise one, and
--     the contract only requires ids and displays to stay parallel — so both
--     sides are sorted before comparison.
--   * the DISPLAY side entirely. A display is the label a value carried at the
--     moment of the event, and renaming a value is ordinary administration: a
--     priority renamed years ago leaves the journal holding the old label for
--     old events and the snapshot holding the new one. Both are correct, so
--     comparing displays reports a rename as a defect. Ids are the comparable
--     side — and where an id is not stable either, the divergence is a real
--     finding (§3.4), not something to relax the check for.

WITH latest_state AS (
    SELECT
        insight_source_id,
        issue_id,
        field_id,
        -- Ordering key, and none of its three parts is optional.
        --
        -- `event_kind` first: a `synthetic_initial` row is the state at creation
        -- and a changelog row a state after an event, so the initial row
        -- precedes any event of the same instant. `_seq` cannot express that —
        -- it is 0 for every changelog row and 1..N for the initial rows, which
        -- sorts them the WRONG way round. An issue whose first event lands on
        -- its own creation timestamp then reads as though the field were still
        -- empty. `retired_field` sorts last: it is stamped when the absence was
        -- observed, which is at or after every event.
        --
        -- `_seq` second, to order the initial rows among themselves — they all
        -- share the creation timestamp.
        --
        -- The event id last, numerically: two changelog rows of one millisecond
        -- both carry `_seq` 0, and as text '101' sorts before '99'.
        argMax(value_ids,      (event_at, {{ task_event_rank('event_kind') }},
                                _seq, toUInt64OrZero(event_id))) AS value_ids,
        argMax(value_displays, (event_at, {{ task_event_rank('event_kind') }},
                                _seq, toUInt64OrZero(event_id))) AS value_displays,
        max(event_at)                            AS latest_event_at
    FROM {{ ref('jira__field_history_derived') }} FINAL
    WHERE field_id != 'created'
      -- `snapshot_diff` is the snapshot's own value written back as an observed
      -- state, so counting it here would compare the snapshot with itself and
      -- report agreement no matter what the events say. The events are the
      -- side under test; a pair that needed a `snapshot_diff` row is exactly a
      -- pair whose events do NOT reach the current value, and it must keep
      -- showing up here.
      AND event_kind != 'snapshot_diff'
      -- A field the catalogue does not contain (§3.2) carries ONE best-effort
      -- row, and the snapshot cannot hold a value for it at all: the snapshot
      -- model joins the catalogue, so an unclassifiable field never reaches it.
      -- Comparing the two is not a weak check, it is a meaningless one — the
      -- relation on the right structurally cannot answer.
      AND event_kind != 'unclassified_field'
      -- `long_text` is exempt: the issue JSON holds an ADF document and the
      -- changelog Jira's rendering of it, so the two content addresses cannot
      -- agree by construction (§8).
      AND field_id NOT IN (
          SELECT field_id FROM {{ ref('jira__task_field_kind') }}
          WHERE field_kind = 'long_text'
      )
    GROUP BY insight_source_id, issue_id, field_id
),

-- How fresh the issue side is. A pair whose history has moved past this point
-- cannot be expected to agree with the snapshot yet.
issue_freshness AS (
    SELECT
        COALESCE(source_id, '')                     AS insight_source_id,
        COALESCE(toString(jira_id), '')             AS issue_id,
        max(_airbyte_extracted_at)                  AS issue_seen_at
    FROM {{ source('bronze_jira', 'jira_issue') }}
    GROUP BY insight_source_id, issue_id
),

snapshot AS (
    SELECT
        insight_source_id,
        issue_id,
        field_id,
        value_ids,
        value_displays
    FROM {{ ref('jira__issue_field_snapshot') }} FINAL
)

SELECT
    h.insight_source_id,
    h.issue_id,
    h.field_id,
    arraySort(h.value_ids)      AS history_ids,
    arraySort(s.value_ids)      AS snapshot_ids,
    -- reported for context only; not compared
    arraySort(h.value_displays) AS history_displays,
    arraySort(s.value_displays) AS snapshot_displays
FROM latest_state AS h
INNER JOIN issue_freshness AS f
    ON f.insight_source_id = h.insight_source_id
   AND f.issue_id = h.issue_id
LEFT JOIN snapshot AS s
    ON s.insight_source_id = h.insight_source_id
   AND s.issue_id = h.issue_id
   AND s.field_id = h.field_id
WHERE h.latest_event_at <= f.issue_seen_at
  AND arraySort(h.value_ids) != arraySort(s.value_ids)
LIMIT 100

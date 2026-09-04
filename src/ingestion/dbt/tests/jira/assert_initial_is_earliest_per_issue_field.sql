-- A synthetic_initial row must be stamped with the issue's creation time.
--
-- That is the property the pipeline owns, and it is what this test asserts. The
-- earlier form compared the initial row against the field's earliest changelog
-- event instead, which conflates two different things:
--
--   * the reconstruct step stamping an initial row wrongly — a pipeline fault,
--     and what we want to catch;
--   * an issue whose own `created` in the source postdates its own changelog —
--     a source fact. It happens on issues that were imported or moved between
--     instances, where `created` was reset while the changelog was carried over.
--     No reconstruction can put the initial row before the creation it is
--     derived from, so flagging it here reports the source as a pipeline bug.
--
-- Comparing against bronze `created` directly separates them: a mismatch is
-- ours, and a source whose creation timestamp is odd no longer looks like our
-- defect. The changelog-order property is still covered — an initial row that
-- is not the earliest row of its (issue, field) shows up as a stamping
-- mismatch here, because every initial row is stamped from one value.
--
-- Jira-only: the comparison needs the vendor's own creation timestamp, so it
-- reads the Jira model rather than the class. Reading silver would compare
-- against whichever arm currently populates it — during the cutover that is
-- still the Rust binary, so the test would be asserting the thing being
-- replaced instead of the replacement.

WITH issue_created AS (
    SELECT
        COALESCE(source_id, '')                                       AS insight_source_id,
        COALESCE(toString(id_readable), '')                           AS id_readable,
        argMax(parseDateTime64BestEffortOrNull(created, 3),
               _airbyte_extracted_at)                                 AS created_at
    FROM {{ source('bronze_jira', 'jira_issue') }}
    WHERE id_readable IS NOT NULL
    GROUP BY insight_source_id, id_readable
),

initial_rows AS (
    SELECT
        insight_source_id,
        data_source,
        id_readable,
        field_id,
        min(event_at) AS first_initial
    FROM {{ ref('jira__field_history_derived') }} FINAL
    WHERE event_kind = 'synthetic_initial'
    GROUP BY insight_source_id, data_source, id_readable, field_id
)

SELECT
    r.insight_source_id,
    r.data_source,
    r.id_readable,
    r.field_id,
    r.first_initial,
    c.created_at
FROM initial_rows AS r
INNER JOIN issue_created AS c
    ON c.insight_source_id = r.insight_source_id
   AND c.id_readable = r.id_readable
-- One-second tolerance: the journal stores milliseconds and the source string is
-- parsed, so an exact equality would fail on rounding alone.
WHERE c.created_at IS NOT NULL
  AND abs(dateDiff('second', r.first_initial, c.created_at)) > 1
LIMIT 100

-- depends_on: {{ ref('jira__bronze_promoted') }}
-- depends_on: {{ ref('jira__task_field_kind') }}
{{ config(
    materialized='table',
    alias='jira_issue_field_snapshot',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={
        'allow_nullable_key': 1,
    },
    query_settings={
        'max_bytes_before_external_group_by': 2000000000,
        'max_bytes_before_external_sort': 2000000000,
    },
    tags=['staging', 'jira']
) }}

-- One row per (issue, field) with the field's current value, for EVERY field the
-- issue actually carries — not a hand-picked list. Consumed by `jira-enrich` to
-- populate `IssueSnapshot.current_fields`, so a field that never appears in the
-- changelog still produces a `synthetic_initial` row.
--
-- Fields are classified by `jira__task_field_kind` and read by the
-- `jira_norm_value` macros; see
-- `connectors/task-tracking/jira/specs/FIELD-HISTORY-IN-DBT.md` §3-§4. Nothing
-- here names a field id: the previous shape of this model enumerated ~10 generic
-- fields by hand, which is why every custom field was missing from the modelled
-- current state and — through `reconstruct_initial` — from the history as well.
--
-- MEMORY. This model reads a JSON column that is gigabytes wide, and earlier
-- shapes of it exhausted the server twice (#1425, #1817). Two rules hold it in
-- place:
--   1. dedup resolves to a raw-id per issue FIRST, in an aggregation that
--      carries only that String — never the JSON. Selecting the JSON into an
--      argMax state, or sorting rows that carry it, puts every issue's payload
--      into one buffer.
--   2. the key/value unpivot happens AFTER that join, so it streams over one
--      row per issue instead of one per bronze emission.
-- A single `argMax` over a wide tuple (the previous shape) also works, but only
-- because the tuple is small; it cannot be extended to all fields.
--
-- The two-pass form has a second benefit: every column is read from ONE chosen
-- bronze row. Independent per-column `argMax` calls can each resolve to a
-- different row when `_airbyte_extracted_at` ties, silently mixing two versions
-- of an issue.

WITH winner AS (
    SELECT
        unique_key,
        argMax(_airbyte_raw_id, _airbyte_extracted_at) AS raw_id
    FROM {{ source('bronze_jira', 'jira_issue') }}
    WHERE unique_key IS NOT NULL
    GROUP BY unique_key
),

-- One row per (issue, field key present in the issue JSON).
unpivoted AS (
    SELECT
        COALESCE(i.source_id, '')                                     AS insight_source_id,
        COALESCE(toString(i.jira_id), '')                             AS issue_id,
        COALESCE(toString(i.id_readable), '')                         AS id_readable,
        COALESCE(parseDateTime64BestEffortOrNull(i.created, 3),
                 toDateTime64(0, 3))                                  AS created_at,
        kv.1                                                          AS field_id,
        kv.2                                                          AS raw_value
    FROM {{ source('bronze_jira', 'jira_issue') }} AS i
    INNER JOIN winner AS w ON i._airbyte_raw_id = w.raw_id
    ARRAY JOIN JSONExtractKeysAndValuesRaw(COALESCE(i.custom_fields_json, '{}')) AS kv
),

classified AS (
    SELECT
        u.*,
        k.field_kind AS field_kind
    FROM unpivoted AS u
    INNER JOIN {{ ref('jira__task_field_kind') }} AS k
        ON k.insight_source_id = u.insight_source_id
       AND k.field_id = u.field_id
    -- A key absent from the field catalogue cannot be classified even in
    -- principle (§3.2). None occur today, and the classifier resolves the whole
    -- catalogue, so this is a guard rather than a filter: an unclassifiable
    -- shape must stop the run, not be normalized by a rule written for
    -- something else.
    -- ClickHouse requires the message to be a constant, so it names where to
    -- look rather than which field: `jira__task_field_kind` is a view, and
    -- `assert_jira_field_kind_covers_catalogue` lists the offending rows.
    WHERE throwIf(k.field_kind = 'UNKNOWN',
                  'unmapped Jira field kind: select from staging.jira__task_field_kind where field_kind is UNKNOWN') = 0

      -- `ignored` keys are containers and derived aggregates, not field state.
      AND k.field_kind != 'ignored'

      -- Only fields the issue actually holds a value for. An absent key already
      -- produced no row; a key present with an empty value means the field
      -- applies to this issue's context but is unset (§6), a state that is
      -- recoverable from bronze on demand and is deliberately not materialized
      -- here — it is four fifths of the key/value pairs and would multiply this
      -- table, and every `synthetic_initial` row derived from it, sevenfold.
      AND u.raw_value NOT IN ('', 'null', '[]', '""', '{}')
)

SELECT
    CAST(concat(
        insight_source_id, '-',
        issue_id, '-',
        field_id
    ) AS String)                                          AS unique_key,
    insight_source_id,
    issue_id,
    id_readable,
    created_at,
    field_id,
    CAST({{ jira_norm_value('field_kind', 'raw_value') }}.1 AS Array(String)) AS value_ids,
    CAST({{ jira_norm_value('field_kind', 'raw_value') }}.2 AS Array(String)) AS value_displays,
    toUnixTimestamp64Milli(now64(3))                      AS _version
FROM classified

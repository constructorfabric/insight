-- depends_on: {{ ref('jira__bronze_promoted') }}
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

{#-
  `allow_nullable_key` is a MergeTree TABLE setting (CREATE TABLE … SETTINGS).
  The `max_bytes_before_external_*` spill knobs are QUERY-execution settings that
  dbt-clickhouse appends to the INSERT … SELECT; they are kept as a safety net for
  the GROUP BY below, which stays far under the threshold at current data volumes.
-#}

-- One row per (issue, field_id) with current value_ids / value_displays.
-- Consumed by `jira-enrich` to populate `IssueSnapshot.current_fields` so
-- synthetic_initial rows can be emitted for every field — even ones that never appear
-- in the changelog.
--
-- All fields extracted from custom_fields_json via JSONExtract (ClickHouse destination
-- nests Jira fields inside a single JSON column rather than top-level columns).
--
-- Dedup MUST be an argMax GROUP BY, not `ORDER BY _airbyte_extracted_at LIMIT 1 BY`:
-- with the sort form the optimizer lifts the JSONExtract projections ABOVE the sort
-- ("lifted up part" in EXPLAIN), so the raw multi-KiB custom_fields_json of every
-- bronze row travels through the sort buffer — and the WITH subquery is inlined into
-- each UNION branch, multiplying that by the number of branches. On virtuozzo
-- (~160k rows, ~5.6 GiB of JSON) that overran the server-wide memory cap even with
-- external sort enabled: the spill's merge phase buffers gigabyte-scale String blocks
-- (Code 241 MEMORY_LIMIT_EXCEEDED in MergingSortedTransform). The aggregation form
-- streams the JSON: extraction happens per block inside argMax and only the small
-- extracted tuple is kept per issue (~0.6 GiB peak, single scan). The ARRAY JOIN
-- unpivot keeps it to one scan instead of one per field.

WITH issue AS (
    SELECT
        COALESCE(source_id, '')                                       AS insight_source_id,
        COALESCE(toString(jira_id), '')                               AS issue_id,
        -- Latest bronze row per issue, projected down to the small extracted tuple.
        -- Tuple indexes: 1 id_readable, 2 created_at, 3/4 status id/name,
        -- 5/6 priority, 7/8 issuetype, 9/10 resolution, 11/12 assignee,
        -- 13/14 reporter, 15 parent_id, 16 project_key, 17 labels_raw, 18 due_date.
        argMax(
            (
                COALESCE(toString(id_readable), ''),
                COALESCE(parseDateTime64BestEffortOrNull(created, 3),
                         toDateTime64(0, 3)),
                JSONExtractString(custom_fields_json, 'status', 'id'),
                JSONExtractString(custom_fields_json, 'status', 'name'),
                JSONExtractString(custom_fields_json, 'priority', 'id'),
                JSONExtractString(custom_fields_json, 'priority', 'name'),
                JSONExtractString(custom_fields_json, 'issuetype', 'id'),
                JSONExtractString(custom_fields_json, 'issuetype', 'name'),
                JSONExtractString(custom_fields_json, 'resolution', 'id'),
                JSONExtractString(custom_fields_json, 'resolution', 'name'),
                JSONExtractString(custom_fields_json, 'assignee', 'accountId'),
                JSONExtractString(custom_fields_json, 'assignee', 'displayName'),
                JSONExtractString(custom_fields_json, 'reporter', 'accountId'),
                JSONExtractString(custom_fields_json, 'reporter', 'displayName'),
                parent_id,
                project_key,
                -- Labels is a JSON array; `JSONExtractString` at an array path
                -- returns '', so take the raw JSON and parse it as Array(String)
                -- in the unpivot below.
                JSONExtractRaw(custom_fields_json, 'labels'),
                due_date
            ),
            _airbyte_extracted_at
        ) AS t
    FROM {{ source('bronze_jira', 'jira_issue') }}
    GROUP BY insight_source_id, issue_id
)

SELECT
       CAST(concat(
           coalesce(insight_source_id, ''), '-',
           coalesce(issue_id, ''), '-',
           coalesce(field_id, '')
       ) AS String)                                                           AS unique_key,
       insight_source_id,
       issue_id,
       t.1                                                                    AS id_readable,
       t.2                                                                    AS created_at,
       f.1                                                                    AS field_id,
       CAST(arrayMap(x -> COALESCE(x, ''), f.2)            AS Array(String)) AS value_ids,
       CAST(arrayMap(x -> COALESCE(x, ''), f.3)            AS Array(String)) AS value_displays,
       toUnixTimestamp64Milli(now64(3))                                      AS _version
FROM issue
ARRAY JOIN [
    ('status',
     if(t.3  = '', [], [t.3]),
     if(t.3  = '', [], [t.4])),
    ('priority',
     if(t.5  = '', [], [t.5]),
     if(t.5  = '', [], [t.6])),
    ('issuetype',
     if(t.7  = '', [], [t.7]),
     if(t.7  = '', [], [t.8])),
    ('resolution',
     if(t.9  = '', [], [t.9]),
     if(t.9  = '', [], [t.10])),
    ('assignee',
     if(t.11 = '', [], [t.11]),
     if(t.11 = '', [], [t.12])),
    ('reporter',
     if(t.13 = '', [], [t.13]),
     if(t.13 = '', [], [t.14])),
    ('project',
     if(t.16 IS NULL OR t.16 = '', [], [t.16]),
     if(t.16 IS NULL OR t.16 = '', [], [t.16])),
    ('parent',
     if(t.15 IS NULL OR t.15 = '', [], [t.15]),
     if(t.15 IS NULL OR t.15 = '', [], [t.15])),
    ('labels',
     JSONExtract(COALESCE(nullIf(t.17, ''), '[]'), 'Array(Nullable(String))'),
     JSONExtract(COALESCE(nullIf(t.17, ''), '[]'), 'Array(Nullable(String))')),

    {#- `story_points` is deliberately omitted here — Jira stores it in an
        instance-specific `customfield_NNNNN` column whose ID must be resolved per
        tenant. Tracked as a gap in
        `docs/components/connectors/task-tracking/specs/task-metrics-map.md`. -#}

    {#- duedate: emit the Jira-native field_id `duedate` (NOT `due_date`): the
        changelog stream uses Jira's `fieldId="duedate"`, jira__task_field_metadata
        carries `duedate` from bronze jira_fields, and the downstream consumer
        `insight.task_issue_current_state` filters `field_id = 'duedate'`. Emitting
        the underscored `due_date` here made snapshot-only due dates (set at
        creation, never changed in the changelog) silently invisible to
        due_date_compliance. -#}
    ('duedate',
     if(t.18 IS NULL OR t.18 = '', [], [toString(t.18)]),
     if(t.18 IS NULL OR t.18 = '', [], [toString(t.18)]))
] AS f

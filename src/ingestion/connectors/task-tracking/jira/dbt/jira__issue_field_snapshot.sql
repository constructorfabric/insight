-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='table',
    alias='jira_issue_field_snapshot',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['staging', 'jira']
) }}

-- One row per (issue, field_id) with current value_ids / value_displays.
-- Consumed by `jira-enrich` to populate `IssueSnapshot.current_fields` so
-- synthetic_initial rows can be emitted for every field — even ones that never appear
-- in the changelog.
--
-- All fields extracted from custom_fields_json via JSONExtract (ClickHouse destination
-- nests Jira fields inside a single JSON column rather than top-level columns).

WITH issue AS (
    SELECT
        COALESCE(source_id, '')                                       AS insight_source_id,
        COALESCE(toString(jira_id), '')                               AS issue_id,
        COALESCE(toString(id_readable), '')                           AS id_readable,
        COALESCE(parseDateTime64BestEffortOrNull(created, 3),
                 toDateTime64(0, 3))                                  AS created_at,
        JSONExtractString(custom_fields_json, 'status', 'id')        AS status_id,
        JSONExtractString(custom_fields_json, 'status', 'name')      AS status_name,
        JSONExtractString(custom_fields_json, 'priority', 'id')      AS priority_id,
        JSONExtractString(custom_fields_json, 'priority', 'name')    AS priority_name,
        JSONExtractString(custom_fields_json, 'issuetype', 'id')     AS issuetype_id,
        JSONExtractString(custom_fields_json, 'issuetype', 'name')   AS issuetype_name,
        JSONExtractString(custom_fields_json, 'resolution', 'id')    AS resolution_id,
        JSONExtractString(custom_fields_json, 'resolution', 'name')  AS resolution_name,
        JSONExtractString(custom_fields_json, 'assignee', 'accountId')    AS assignee_id,
        JSONExtractString(custom_fields_json, 'assignee', 'displayName') AS assignee_name,
        JSONExtractString(custom_fields_json, 'reporter', 'accountId')    AS reporter_id,
        JSONExtractString(custom_fields_json, 'reporter', 'displayName') AS reporter_name,
        parent_id,
        project_key,
        -- Labels is a JSON array; `JSONExtractString` at an array path returns '',
        -- so take the raw JSON and let the UNION branch parse it as Array(String).
        JSONExtractRaw(custom_fields_json, 'labels')                 AS labels_raw,
        due_date
    FROM (
        SELECT * FROM {{ source('bronze_jira', 'jira_issue') }}
        ORDER BY _airbyte_extracted_at DESC
        LIMIT 1 BY source_id, jira_id
    ) AS ji
)

SELECT
       CAST(concat(
           coalesce(insight_source_id, ''), '-',
           coalesce(issue_id, ''), '-',
           coalesce(field_id, '')
       ) AS String)                                                           AS unique_key,
       insight_source_id, issue_id, id_readable, created_at, field_id,
       CAST(arrayMap(x -> COALESCE(x, ''), value_ids)      AS Array(String)) AS value_ids,
       CAST(arrayMap(x -> COALESCE(x, ''), value_displays) AS Array(String)) AS value_displays,
       toUnixTimestamp64Milli(now64(3))                                      AS _version
FROM (
    -- status
    SELECT i.insight_source_id, i.issue_id, i.id_readable, i.created_at,
           'status' AS field_id,
           if(i.status_id = '', [], [i.status_id])      AS value_ids,
           if(i.status_id = '', [], [i.status_name])     AS value_displays
    FROM issue i

    UNION ALL

    -- priority
    SELECT i.insight_source_id, i.issue_id, i.id_readable, i.created_at,
           'priority',
           if(i.priority_id = '', [], [i.priority_id]),
           if(i.priority_id = '', [], [i.priority_name])
    FROM issue i

    UNION ALL

    -- issuetype
    SELECT i.insight_source_id, i.issue_id, i.id_readable, i.created_at,
           'issuetype',
           if(i.issuetype_id = '', [], [i.issuetype_id]),
           if(i.issuetype_id = '', [], [i.issuetype_name])
    FROM issue i

    UNION ALL

    -- resolution
    SELECT i.insight_source_id, i.issue_id, i.id_readable, i.created_at,
           'resolution',
           if(i.resolution_id = '', [], [i.resolution_id]),
           if(i.resolution_id = '', [], [i.resolution_name])
    FROM issue i

    UNION ALL

    -- assignee
    SELECT i.insight_source_id, i.issue_id, i.id_readable, i.created_at,
           'assignee',
           if(i.assignee_id = '', [], [i.assignee_id]),
           if(i.assignee_id = '', [], [i.assignee_name])
    FROM issue i

    UNION ALL

    -- reporter
    SELECT i.insight_source_id, i.issue_id, i.id_readable, i.created_at,
           'reporter',
           if(i.reporter_id = '', [], [i.reporter_id]),
           if(i.reporter_id = '', [], [i.reporter_name])
    FROM issue i

    UNION ALL

    -- project
    SELECT i.insight_source_id, i.issue_id, i.id_readable, i.created_at,
           'project',
           if(i.project_key IS NULL OR i.project_key = '', [], [i.project_key]),
           if(i.project_key IS NULL OR i.project_key = '', [], [i.project_key])
    FROM issue i

    UNION ALL

    -- parent
    SELECT i.insight_source_id, i.issue_id, i.id_readable, i.created_at,
           'parent',
           if(i.parent_id IS NULL OR i.parent_id = '', [], [i.parent_id]),
           if(i.parent_id IS NULL OR i.parent_id = '', [], [i.parent_id])
    FROM issue i

    UNION ALL

    -- labels. `labels_raw` is the raw JSON array from custom_fields_json.
    SELECT i.insight_source_id, i.issue_id, i.id_readable, i.created_at,
           'labels',
           JSONExtract(COALESCE(nullIf(i.labels_raw, ''), '[]'), 'Array(String)'),
           JSONExtract(COALESCE(nullIf(i.labels_raw, ''), '[]'), 'Array(String)')
    FROM issue i

    -- NOTE: `story_points` is deliberately omitted here — Jira stores it in an
    -- instance-specific `customfield_NNNNN` column whose ID must be resolved per
    -- tenant. Tracked as a gap in `docs/components/connectors/task-tracking/specs/task-metrics-map.md`.

    UNION ALL

    -- due_date
    SELECT i.insight_source_id, i.issue_id, i.id_readable, i.created_at,
           'due_date',
           if(i.due_date IS NULL OR i.due_date = '', [], [toString(i.due_date)]),
           if(i.due_date IS NULL OR i.due_date = '', [], [toString(i.due_date)])
    FROM issue i
)

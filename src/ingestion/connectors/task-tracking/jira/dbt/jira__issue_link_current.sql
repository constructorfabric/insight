{{ config(
    materialized='table',
    alias='jira__issue_link_current',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    query_settings={'join_use_nulls': 1},
    tags=['staging', 'jira']
) }}

-- MEMORY. Same two-pass read as `jira__issue_field_snapshot`: the dedup
-- resolves a raw id per issue in an aggregation carrying only that String, and
-- the JSON is read after joining back, one row per issue.

WITH winner AS (
    SELECT
        source_id,
        jira_id,
        argMax(_airbyte_raw_id, _airbyte_extracted_at) AS raw_id
    FROM {{ source('bronze_jira', 'jira_issue') }}
    GROUP BY source_id, jira_id
),

issues AS (
    SELECT
        CAST(i.source_id AS String)                                   AS insight_source_id,
        CAST(i.jira_id AS String)                                     AS issue_id,
        COALESCE(toString(i.id_readable), '')                         AS id_readable,
        toDateTime64(i._airbyte_extracted_at, 3)                      AS extracted_at,
        JSONExtractArrayRaw(COALESCE(i.custom_fields_json, '{}'), 'issuelinks') AS link_objects,
        JSONExtractRaw(COALESCE(i.custom_fields_json, '{}'), 'parent')          AS parent_object
    FROM {{ source('bronze_jira', 'jira_issue') }} AS i
    INNER JOIN winner AS w ON i._airbyte_raw_id = w.raw_id
),

link_rows AS (
    SELECT
        insight_source_id,
        issue_id,
        id_readable,
        extracted_at,
        arrayJoin(arrayConcat(
            arrayMap(link -> ('issuelink', link), link_objects),
            if(JSONExtractString(parent_object, 'id') != ''
                   OR JSONExtractString(parent_object, 'key') != '',
               [('parent', parent_object)], [])
        )) AS link_object
    FROM issues
),

-- `outwardIssue` on this issue's link means this issue holds the type's
-- outward relation ("blocks"); `inwardIssue`, its inward one ("is blocked by").
parsed_links AS (
    SELECT
        insight_source_id,
        issue_id,
        id_readable,
        extracted_at,
        link_object.1 AS kind,
        link_object.2 AS link,
        if(kind = 'parent', '', JSONExtractString(link, 'id')) AS link_id,
        if(kind = 'parent', '', JSONExtractString(link, 'type', 'id')) AS type_id,
        if(kind = 'parent', '', JSONExtractString(link, 'type', 'name')) AS type_name,
        if(kind = 'parent', '', JSONExtractString(link, 'type', 'outward')) AS type_outward,
        if(kind = 'parent', '', JSONExtractString(link, 'type', 'inward')) AS type_inward,
        if(kind = 'parent', '', if(JSONHas(link, 'outwardIssue'), 'outward', 'inward')) AS direction,
        if(kind = 'parent', 'parent', if(direction = 'outward', type_outward, type_inward)) AS relation,
        if(kind = 'parent', JSONExtractString(link, 'id'), {{ jira_json_link_issue_id('link') }}) AS target_id,
        if(kind = 'parent', JSONExtractString(link, 'key'), {{ jira_json_link_key('link') }}) AS target_key
    FROM link_rows
)

SELECT
    CAST(concat(insight_source_id, '-', issue_id, '-', kind, '-', link_id, '-', target_id, '-', target_key) AS String) AS unique_key,
    insight_source_id,
    issue_id,
    id_readable,
    kind,
    link_id,
    type_id,
    type_name,
    type_outward,
    type_inward,
    direction,
    relation,
    target_id,
    target_key,
    extracted_at,
    toUnixTimestamp64Milli(now64(3)) AS _version
FROM parsed_links

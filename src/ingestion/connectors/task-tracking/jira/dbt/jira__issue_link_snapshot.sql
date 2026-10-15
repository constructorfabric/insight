{{ config(
    materialized='table',
    alias='jira__issue_link_snapshot',
    schema='staging',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['staging', 'jira']
) }}

-- The links and the parent each issue holds now, one row per link object, with
-- the linked issue named by its immutable id and its key. Read by
-- `jira__task_links`, which seeds the links no changelog event reaches, and by
-- `jira__issue_key_ids`, for which every row is a key the linked issue has.
--
-- MEMORY. Same two-pass read as `jira__issue_field_snapshot`: the dedup
-- resolves a raw id per issue in an aggregation carrying only that String, and
-- the JSON is read after joining back, one row per issue.

WITH winner AS (
    SELECT
        source_id,
        jira_id,
        argMax(_airbyte_raw_id, _airbyte_extracted_at) AS raw_id
    FROM {{ source('bronze_jira', 'jira_issue') }}
    WHERE jira_id IS NOT NULL
    GROUP BY source_id, jira_id
),

issues AS (
    SELECT
        COALESCE(i.source_id, '')                                     AS insight_source_id,
        COALESCE(toString(i.jira_id), '')                             AS issue_id,
        COALESCE(toString(i.id_readable), '')                         AS id_readable,
        toDateTime64(i._airbyte_extracted_at, 3)                      AS extracted_at,
        JSONExtractArrayRaw(COALESCE(i.custom_fields_json, '{}'), 'issuelinks') AS link_objects,
        JSONExtractRaw(COALESCE(i.custom_fields_json, '{}'), 'parent')          AS parent_object
    FROM {{ source('bronze_jira', 'jira_issue') }} AS i
    INNER JOIN winner AS w ON i._airbyte_raw_id = w.raw_id
),

-- `outwardIssue` on this issue's link means this issue holds the type's
-- outward relation ("blocks"); `inwardIssue`, its inward one ("is blocked by").
issue_links AS (
    SELECT
        insight_source_id,
        issue_id,
        id_readable,
        extracted_at,
        'issuelink'                                                   AS kind,
        JSONExtractString(link, 'id')                                 AS link_id,
        JSONExtractString(link, 'type', 'name')                       AS type_name,
        JSONExtractString(link, 'type', 'outward')                    AS type_outward,
        JSONExtractString(link, 'type', 'inward')                     AS type_inward,
        if(JSONHas(link, 'outwardIssue'), 'outward', 'inward')        AS direction,
        if(direction = 'outward', type_outward, type_inward)          AS relation,
        {{ jira_json_link_issue_id('link') }}                         AS target_id,
        {{ jira_json_link_key('link') }}                              AS target_key
    FROM issues
    ARRAY JOIN link_objects AS link
),

parents AS (
    SELECT
        insight_source_id,
        issue_id,
        id_readable,
        extracted_at,
        'parent'                                                      AS kind,
        ''                                                            AS link_id,
        ''                                                            AS type_name,
        ''                                                            AS type_outward,
        ''                                                            AS type_inward,
        ''                                                            AS direction,
        'parent'                                                      AS relation,
        JSONExtractString(parent_object, 'id')                        AS target_id,
        JSONExtractString(parent_object, 'key')                       AS target_key
    FROM issues
    WHERE JSONExtractString(parent_object, 'id') != ''
       OR JSONExtractString(parent_object, 'key') != ''
),

held AS (
    SELECT * FROM issue_links
    UNION ALL
    SELECT * FROM parents
)

SELECT
    CAST(concat(insight_source_id, '-', issue_id, '-', kind, '-', link_id, '-', target_id, '-', target_key) AS String) AS unique_key,
    insight_source_id,
    issue_id,
    id_readable,
    kind,
    link_id,
    type_name,
    type_outward,
    type_inward,
    direction,
    relation,
    target_id,
    target_key,
    extracted_at,
    toUnixTimestamp64Milli(now64(3))                                  AS _version
FROM held

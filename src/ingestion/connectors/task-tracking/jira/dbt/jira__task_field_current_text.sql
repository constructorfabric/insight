-- depends_on: {{ ref('jira__task_field_kind') }}
{{ config(
    materialized='table',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    tags=['staging', 'jira', 'silver:class_task_field_current_text']
) }}

-- The current body of each long-text field of an issue, as the issue JSON holds
-- it (`adf_json`): a reference into `jira__task_field_text`. The history's newest
-- reference is Jira's changelog rendering once the field has changed, so this is
-- the only way to the exact current document. An empty field has no row.

WITH long_text_fields AS (
    SELECT insight_source_id, field_id
    FROM {{ ref('jira__task_field_kind') }}
    WHERE field_kind = 'long_text'
),

issue_winner AS (
    -- Read-time dedup by the issue's stable key within a source.
    SELECT source_id, jira_id, argMax(_airbyte_raw_id, _airbyte_extracted_at) AS raw_id
    FROM {{ source('bronze_jira', 'jira_issue') }}
    GROUP BY source_id, jira_id
),

current_bodies AS (
    SELECT
        CAST(i.source_id AS String)                       AS insight_source_id,
        CAST(i.jira_id AS String)                         AS issue_id,
        kv.1                                              AS field_id,
        kv.2                                              AS content
    FROM {{ source('bronze_jira', 'jira_issue') }} AS i
    INNER JOIN issue_winner AS w ON i._airbyte_raw_id = w.raw_id
    ARRAY JOIN JSONExtractKeysAndValuesRaw(COALESCE(i.custom_fields_json, '{}')) AS kv
    INNER JOIN long_text_fields AS f
        ON f.insight_source_id = CAST(i.source_id AS String)
       AND f.field_id = kv.1
    WHERE kv.2 NOT IN ('', 'null', '""', '{}')
)

SELECT
    concat(insight_source_id, '-jira-', issue_id, '-', field_id) AS unique_key,
    insight_source_id,
    CAST('jira' AS String)                                AS data_source,
    issue_id,
    field_id,
    {{ task_text_id('content') }}                         AS text_id,
    toUInt64(toUnixTimestamp64Milli(now64(3)))            AS _version
FROM current_bodies

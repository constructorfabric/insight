-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='incremental',
    alias='jira__task_comments',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['jira', 'silver:class_task_comments']
) }}

-- `body` is raw ADF JSON at Bronze level; plaintext extraction deferred.

SELECT
    c.unique_key                                        AS unique_key,
    c.source_id                                         AS insight_source_id,
    CAST('jira' AS String)                              AS data_source,
    c.comment_id                                        AS comment_id,
    c.id_readable                                       AS id_readable,
    c.author_account_id                                 AS author_id,
    parseDateTime64BestEffortOrNull(c.created, 3)       AS created_at,
    parseDateTime64BestEffortOrNull(c.updated, 3)       AS updated_at,
    c.body                                              AS body,
    toUInt8(0)                                          AS is_deleted,
    toUnixTimestamp64Milli(now64(3))                    AS _version
FROM (
    SELECT * FROM {{ source('bronze_jira', 'jira_comments') }}
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY _airbyte_raw_id
) c

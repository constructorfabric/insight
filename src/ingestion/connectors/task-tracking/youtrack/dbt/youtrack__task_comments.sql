-- depends_on: {{ ref('youtrack__bronze_promoted') }}
{{ config(
    materialized='incremental',
    alias='youtrack__task_comments',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['youtrack', 'silver:class_task_comments']
) }}

-- Per-source staging projection; unioned into `silver.class_task_comments`.
-- Comments come from an append/incremental substream, so this is an incremental
-- RMT table (mirrors jira__task_comments), not a view. Bronze is deduped by
-- `_airbyte_raw_id` at read time before append; RMT(_version) + union_by_tag
-- collapse any residual duplicates.
--
-- Bronze `youtrack_comments` carries only the parent issue's internal id
-- (`youtrack_id`), not the human `id_readable`; resolved via a deduped join to
-- `youtrack_issue`, falling back to the internal id. Timestamps are epoch-ms.

SELECT
    c.unique_key                                            AS unique_key,
    c.source_id                                             AS insight_source_id,
    CAST('youtrack' AS String)                              AS data_source,
    toString(c.comment_id)                                  AS comment_id,
    COALESCE(i.id_readable, toString(c.youtrack_id))        AS id_readable,
    c.author_id                                             AS author_id,
    fromUnixTimestamp64Milli(toInt64OrNull(toString(c.created)))  AS created_at,
    fromUnixTimestamp64Milli(toInt64OrNull(toString(c.updated)))  AS updated_at,
    c.text                                                  AS body,
    if(c.deleted IS NULL, toUInt8(0), toUInt8(c.deleted))   AS is_deleted,
    toUnixTimestamp64Milli(now64(3))                        AS _version
FROM (
    SELECT * FROM {{ source('bronze_youtrack', 'youtrack_comments') }}
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY _airbyte_raw_id
) c
LEFT JOIN (
    SELECT source_id, youtrack_id, id_readable
    FROM {{ source('bronze_youtrack', 'youtrack_issue') }}
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY source_id, youtrack_id
) i ON i.source_id = c.source_id AND i.youtrack_id = c.youtrack_id

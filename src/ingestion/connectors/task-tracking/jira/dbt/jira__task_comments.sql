-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='incremental',
    alias='jira__task_comments',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['jira', 'staging', 'silver:class_task_comments']
) }}

-- `body` is raw ADF JSON at Bronze level; plaintext extraction deferred.
-- State (including is_deleted — specs/DELETION-AND-VISIBILITY.md) is computed
-- once in jira__comment_state; this is the class-contract projection.

SELECT
    s.unique_key                                        AS unique_key,
    s.source_id                                         AS insight_source_id,
    CAST('jira' AS String)                              AS data_source,
    s.comment_id                                        AS comment_id,
    s.id_readable                                       AS id_readable,
    s.author_id                                         AS author_id,
    s.created_at                                        AS created_at,
    s.edited_at                                         AS updated_at,
    s.body                                              AS body,
    toNullable(s.is_deleted)                            AS is_deleted,
    toUnixTimestamp64Milli(now64(3))                    AS _version
FROM {{ ref('jira__comment_state') }} AS s FINAL

-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='incremental',
    alias='jira__task_availability',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['jira', 'silver', 'silver:class_task_availability'],
    query_settings={'join_use_nulls': 1}
) }}

-- Latest availability per issue for the silver class contract. The state
-- table is the current truth; availability_changed_at comes from the SCD2
-- snapshot's latest version (_tracked_at = when the current state was first
-- detected), which the recomputed state table cannot know.

SELECT
    concat(s.tenant_id, '-', s.source_id, '-issue-', s.jira_id) AS unique_key,
    s.tenant_id                                         AS tenant_id,
    s.source_id                                         AS insight_source_id,
    CAST('jira' AS String)                              AS data_source,
    CAST('issue' AS String)                             AS entity_kind,
    toString(s.jira_id)                                 AS entity_id,
    s.id_readable                                       AS id_readable,
    s.availability                                      AS availability,
    s.last_seen_at                                      AS last_seen_at,
    sn._tracked_at                                      AS availability_changed_at,
    toUnixTimestamp64Milli(now64(3))                    AS _version
FROM {{ ref('jira__issue_availability_state') }} AS s FINAL
LEFT JOIN (
    SELECT
        unique_key,
        _tracked_at
    FROM {{ ref('jira__issue_availability_snapshot') }}
    ORDER BY _tracked_at DESC
    LIMIT 1 BY unique_key
) AS sn
    ON sn.unique_key = s.unique_key

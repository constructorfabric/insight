{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['hubspot', 'silver:class_crm_activities']
) }}

-- Live + archived sibling Bronze tables UNION ALL'd per engagement type;
-- meetings has no archived sibling because HubSpot returns 400 on
-- /crm/v3/objects/meetings?archived=true ("Paging through deleted objects
-- is not yet supported"). `_version = greatest(updatedAt, archivedAt)` so
-- an archive event outranks the prior live update under ReplacingMergeTree.
WITH calls AS (
    {% for tbl in ['engagements_calls', 'engagements_calls_archived'] %}
    SELECT
        tenant_id,
        source_id,
        unique_key,
        id                                              AS activity_id,
        'call'                                          AS activity_type,
        properties_hubspot_owner_id                     AS owner_id,
        nullIf(arrayElement(JSONExtract(coalesce(associations_contacts, '[]'), 'Array(String)'), 1), '')  AS contact_id,
        nullIf(arrayElement(JSONExtract(coalesce(associations_deals, '[]'), 'Array(String)'), 1), '')     AS deal_id,
        nullIf(arrayElement(JSONExtract(coalesce(associations_companies, '[]'), 'Array(String)'), 1), '') AS account_id,
        -- Deterministic fallback so timestamp never NULLs: try hs_timestamp,
        -- then createdAt, finally epoch 0 so Silver schema `not_null` holds.
        coalesce(
            parseDateTime64BestEffortOrNull(toString(properties_hs_timestamp), 3),
            createdAt,
            toDateTime64(0, 3)
        )                                               AS timestamp,
        -- hs_call_duration is in milliseconds. Preserve NULLs so Silver can
        -- distinguish "unknown duration" from a real zero-duration call.
        CASE
            WHEN properties_hs_call_duration IS NULL THEN NULL
            ELSE intDiv(toInt64OrNull(properties_hs_call_duration), 1000)
        END                                             AS duration_seconds,
        properties_hs_call_disposition                  AS outcome,
        toJSONString(map(
            'title',          coalesce(toString(properties_hs_call_title), ''),
            'direction',      coalesce(toString(properties_hs_call_direction), ''),
            'archived',       toString(coalesce(archived, false))
        ))                                              AS metadata,
        createdAt                                       AS created_at,
        data_source,
        greatest(
            coalesce(toUnixTimestamp64Milli(updatedAt), 0),
            coalesce(toUnixTimestamp64Milli(archivedAt), 0)
        ) AS _version
    FROM {{ source('bronze_hubspot', tbl) }}
    {% if not loop.last %}UNION ALL{% endif %}
    {% endfor %}
),
emails AS (
    {% for tbl in ['engagements_emails', 'engagements_emails_archived'] %}
    SELECT
        tenant_id,
        source_id,
        unique_key,
        id                                              AS activity_id,
        'email'                                         AS activity_type,
        properties_hubspot_owner_id                     AS owner_id,
        nullIf(arrayElement(JSONExtract(coalesce(associations_contacts, '[]'), 'Array(String)'), 1), '')  AS contact_id,
        nullIf(arrayElement(JSONExtract(coalesce(associations_deals, '[]'), 'Array(String)'), 1), '')     AS deal_id,
        nullIf(arrayElement(JSONExtract(coalesce(associations_companies, '[]'), 'Array(String)'), 1), '') AS account_id,
        -- Deterministic fallback so timestamp never NULLs: try hs_timestamp,
        -- then createdAt, finally epoch 0 so Silver schema `not_null` holds.
        coalesce(
            parseDateTime64BestEffortOrNull(toString(properties_hs_timestamp), 3),
            createdAt,
            toDateTime64(0, 3)
        )                                               AS timestamp,
        CAST(NULL AS Nullable(Int64))                   AS duration_seconds,
        properties_hs_email_status                      AS outcome,
        toJSONString(map(
            'subject',        coalesce(toString(properties_hs_email_subject), ''),
            'direction',      coalesce(toString(properties_hs_email_direction), ''),
            'archived',       toString(coalesce(archived, false))
        ))                                              AS metadata,
        createdAt                                       AS created_at,
        data_source,
        greatest(
            coalesce(toUnixTimestamp64Milli(updatedAt), 0),
            coalesce(toUnixTimestamp64Milli(archivedAt), 0)
        ) AS _version
    FROM {{ source('bronze_hubspot', tbl) }}
    {% if not loop.last %}UNION ALL{% endif %}
    {% endfor %}
),
meetings AS (
    -- engagements_meetings_archived intentionally absent: HubSpot returns
    -- HTTP 400 on /crm/v3/objects/meetings?archived=true.
    SELECT
        tenant_id,
        source_id,
        unique_key,
        id                                              AS activity_id,
        'meeting'                                       AS activity_type,
        properties_hubspot_owner_id                     AS owner_id,
        nullIf(arrayElement(JSONExtract(coalesce(associations_contacts, '[]'), 'Array(String)'), 1), '')  AS contact_id,
        nullIf(arrayElement(JSONExtract(coalesce(associations_deals, '[]'), 'Array(String)'), 1), '')     AS deal_id,
        nullIf(arrayElement(JSONExtract(coalesce(associations_companies, '[]'), 'Array(String)'), 1), '') AS account_id,
        -- Deterministic fallback: meeting_start → hs_timestamp → createdAt → epoch 0.
        -- Properties come as Nullable(String); parse before use.
        coalesce(
            parseDateTime64BestEffortOrNull(toString(properties_hs_meeting_start_time), 3),
            parseDateTime64BestEffortOrNull(toString(properties_hs_timestamp), 3),
            createdAt,
            toDateTime64(0, 3)
        )                                               AS timestamp,
        -- Meeting duration in seconds. Preserve NULLs so "unknown duration"
        -- is distinguishable from zero-length.
        CASE
            WHEN properties_hs_meeting_end_time IS NOT NULL
             AND properties_hs_meeting_start_time IS NOT NULL
            THEN intDiv(
                toUnixTimestamp64Milli(parseDateTime64BestEffortOrNull(toString(properties_hs_meeting_end_time), 3))
                  - toUnixTimestamp64Milli(parseDateTime64BestEffortOrNull(toString(properties_hs_meeting_start_time), 3)),
                1000
            )
            ELSE NULL
        END                                             AS duration_seconds,
        properties_hs_meeting_outcome                   AS outcome,
        toJSONString(map(
            'title',          coalesce(toString(properties_hs_meeting_title), ''),
            'location',       coalesce(toString(properties_hs_meeting_location), ''),
            'archived',       toString(coalesce(archived, false))
        ))                                              AS metadata,
        createdAt                                       AS created_at,
        data_source,
        coalesce(
            toUnixTimestamp64Milli(updatedAt),
            0
        ) AS _version
    FROM {{ source('bronze_hubspot', 'engagements_meetings') }}
),
tasks AS (
    {% for tbl in ['engagements_tasks', 'engagements_tasks_archived'] %}
    SELECT
        tenant_id,
        source_id,
        unique_key,
        id                                              AS activity_id,
        'task'                                          AS activity_type,
        properties_hubspot_owner_id                     AS owner_id,
        nullIf(arrayElement(JSONExtract(coalesce(associations_contacts, '[]'), 'Array(String)'), 1), '')  AS contact_id,
        nullIf(arrayElement(JSONExtract(coalesce(associations_deals, '[]'), 'Array(String)'), 1), '')     AS deal_id,
        nullIf(arrayElement(JSONExtract(coalesce(associations_companies, '[]'), 'Array(String)'), 1), '') AS account_id,
        -- Deterministic fallback so timestamp never NULLs: try hs_timestamp,
        -- then createdAt, finally epoch 0 so Silver schema `not_null` holds.
        coalesce(
            parseDateTime64BestEffortOrNull(toString(properties_hs_timestamp), 3),
            createdAt,
            toDateTime64(0, 3)
        )                                               AS timestamp,
        CAST(NULL AS Nullable(Int64))                   AS duration_seconds,
        properties_hs_task_status                       AS outcome,
        toJSONString(map(
            'subject',        coalesce(toString(properties_hs_task_subject), ''),
            'priority',       coalesce(toString(properties_hs_task_priority), ''),
            'type',           coalesce(toString(properties_hs_task_type), ''),
            'archived',       toString(coalesce(archived, false))
        ))                                              AS metadata,
        createdAt                                       AS created_at,
        data_source,
        greatest(
            coalesce(toUnixTimestamp64Milli(updatedAt), 0),
            coalesce(toUnixTimestamp64Milli(archivedAt), 0)
        ) AS _version
    FROM {{ source('bronze_hubspot', tbl) }}
    {% if not loop.last %}UNION ALL{% endif %}
    {% endfor %}
),
combined AS (
    SELECT * FROM calls
    UNION ALL SELECT * FROM emails
    UNION ALL SELECT * FROM meetings
    UNION ALL SELECT * FROM tasks
)
{% if is_incremental() %}
SELECT combined.*
FROM combined
LEFT JOIN (
    SELECT tenant_id, source_id, max(_version) AS hwm
    FROM {{ this }}
    GROUP BY tenant_id, source_id
) w
  ON w.tenant_id = combined.tenant_id AND w.source_id = combined.source_id
WHERE combined._version > coalesce(w.hwm, 0)
{% else %}
SELECT * FROM combined
{% endif %}

{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['salesforce', 'silver:class_crm_activities']
) }}

WITH tasks AS (
    SELECT
        tenant_id,
        source_id,
        unique_key,
        Id                                          AS activity_id,
        CASE
            WHEN CallType IS NOT NULL THEN 'call'
            WHEN TaskSubtype = 'Email' THEN 'email'
            ELSE 'task'
        END                                         AS activity_type,
        OwnerId                                     AS owner_id,
        WhoId                                       AS contact_id,
        CASE WHEN startsWith(coalesce(WhatId, ''), '006') THEN WhatId
             ELSE NULL END                          AS deal_id,
        CASE WHEN startsWith(coalesce(WhatId, ''), '001') THEN WhatId
             ELSE NULL END                          AS account_id,
        parseDateTimeBestEffort(
            coalesce(toString(ActivityDate), toString(CreatedDate))
        )                                           AS timestamp,
        CASE WHEN CallType IS NOT NULL AND CallDurationInSeconds IS NOT NULL
             THEN toInt64(CallDurationInSeconds)
             ELSE NULL END                          AS duration_seconds,
        CAST(Status AS Nullable(String))            AS outcome,
        toJSONString(map(
            'Subject',     coalesce(toString(Subject), ''),
            'Priority',    coalesce(toString(Priority), ''),
            'TaskSubtype', coalesce(toString(TaskSubtype), ''),
            'CallType',    coalesce(toString(CallType), ''),
            'IsDeleted',   toString(coalesce(IsDeleted, false))
        ))                                          AS metadata,
        custom_fields,
        parseDateTimeBestEffort(CreatedDate)        AS created_at,
        data_source,
        toUnixTimestamp64Milli(
            parseDateTime64BestEffort(SystemModstamp)
        )                                           AS _version
    FROM {{ source('bronze_salesforce', 'Task') }}
),
events AS (
    SELECT
        tenant_id,
        source_id,
        unique_key,
        Id                                          AS activity_id,
        CASE
            WHEN EventSubtype IS NULL OR EventSubtype = 'Event' THEN 'event'
            ELSE 'meeting'
        END                                         AS activity_type,
        OwnerId                                     AS owner_id,
        WhoId                                       AS contact_id,
        CASE WHEN startsWith(coalesce(WhatId, ''), '006') THEN WhatId
             ELSE NULL END                          AS deal_id,
        CASE WHEN startsWith(coalesce(WhatId, ''), '001') THEN WhatId
             ELSE NULL END                          AS account_id,
        parseDateTimeBestEffort(
            coalesce(toString(StartDateTime), toString(ActivityDate), toString(CreatedDate))
        )                                           AS timestamp,
        toInt64OrNull(DurationInMinutes) * 60       AS duration_seconds,
        CAST(NULL AS Nullable(String))              AS outcome,
        toJSONString(map(
            'Subject',      coalesce(toString(Subject), ''),
            'Location',     coalesce(toString(Location), ''),
            'EndDateTime',  coalesce(toString(EndDateTime), ''),
            'EventSubtype', coalesce(toString(EventSubtype), ''),
            'IsDeleted',    toString(coalesce(IsDeleted, false))
        ))                                          AS metadata,
        custom_fields,
        parseDateTimeBestEffort(CreatedDate)        AS created_at,
        data_source,
        toUnixTimestamp64Milli(
            parseDateTime64BestEffort(SystemModstamp)
        )                                           AS _version
    FROM {{ source('bronze_salesforce', 'Event') }}
),
combined AS (
    SELECT * FROM tasks
    UNION ALL
    SELECT * FROM events
)
SELECT * FROM combined
{% if is_incremental() %}
WHERE _version > coalesce((SELECT max(_version) FROM {{ this }}), 0)
{% endif %}

-- depends_on: {{ ref('salesforce__bronze_promoted') }}
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
        -- Rep who logged the activity (universal SF audit field). Parallels
        -- HubSpot's `properties_hs_created_by_user_id`; gold attributes
        -- non-call activities by this column.
        CAST(CreatedById AS Nullable(String))       AS created_by_user_id,
        -- WhoId is polymorphic: 003-prefixed = Contact, 00Q-prefixed = Lead.
        -- Leads have no silver class; keep contact_id referentially valid.
        CASE WHEN startsWith(coalesce(WhoId, ''), '003') THEN WhoId
             ELSE NULL END                          AS contact_id,
        CASE WHEN startsWith(coalesce(WhatId, ''), '006') THEN WhatId
             ELSE NULL END                          AS deal_id,
        CASE WHEN startsWith(coalesce(WhatId, ''), '001') THEN WhatId
             ELSE NULL END                          AS account_id,
        coalesce(
            CAST(ActivityDate AS Nullable(DateTime64(3))),
            CreatedDate
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
        CreatedDate                                 AS created_at,
        data_source,
        coalesce(toUnixTimestamp64Milli(SystemModstamp), 0) AS _version
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
        -- Rep who logged the activity (universal SF audit field). Parallels
        -- HubSpot's `properties_hs_created_by_user_id`; gold attributes
        -- non-call activities by this column.
        CAST(CreatedById AS Nullable(String))       AS created_by_user_id,
        -- WhoId is polymorphic: 003-prefixed = Contact, 00Q-prefixed = Lead.
        -- Leads have no silver class; keep contact_id referentially valid.
        CASE WHEN startsWith(coalesce(WhoId, ''), '003') THEN WhoId
             ELSE NULL END                          AS contact_id,
        CASE WHEN startsWith(coalesce(WhatId, ''), '006') THEN WhatId
             ELSE NULL END                          AS deal_id,
        CASE WHEN startsWith(coalesce(WhatId, ''), '001') THEN WhatId
             ELSE NULL END                          AS account_id,
        coalesce(
            StartDateTime,
            CAST(ActivityDate AS Nullable(DateTime64(3))),
            CreatedDate
        )                                           AS timestamp,
        DurationInMinutes * 60                      AS duration_seconds,
        CAST(NULL AS Nullable(String))              AS outcome,
        toJSONString(map(
            'Subject',      coalesce(toString(Subject), ''),
            'Location',     coalesce(toString(Location), ''),
            'EndDateTime',  coalesce(toString(EndDateTime), ''),
            'EventSubtype', coalesce(toString(EventSubtype), ''),
            'IsDeleted',    toString(coalesce(IsDeleted, false))
        ))                                          AS metadata,
        custom_fields,
        CreatedDate                                 AS created_at,
        data_source,
        coalesce(toUnixTimestamp64Milli(SystemModstamp), 0) AS _version
    FROM {{ source('bronze_salesforce', 'Event') }}
),
combined AS (
    SELECT * FROM tasks
    UNION ALL
    SELECT * FROM events
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

-- depends_on: {{ ref('youtrack__bronze_promoted') }}
{{ config(
    materialized='view',
    alias='youtrack__task_users',
    schema='staging',
    tags=['youtrack', 'silver:class_task_users']
) }}

-- Per-source staging projection; unioned into `silver.class_task_users` via `union_by_tag`.
-- Anchor for identity resolution. `email` may be null when YouTrack Hub privacy
-- hides it; identity resolution falls back to login/id (youtrack DESIGN
-- `cpt-insightspec-principle-youtrack-identity-by-email`).

SELECT
    u.unique_key                                            AS unique_key,
    u.tenant_id                                             AS tenant_id,
    u.source_id                                             AS insight_source_id,
    CAST('youtrack' AS String)                              AS data_source,
    toString(u.user_id)                                     AS user_id,
    u.email                                                 AS email,
    u.full_name                                             AS display_name,
    u.login                                                 AS username,
    multiIf(u.guest IS NULL, CAST(NULL AS Nullable(String)),
            u.guest, 'guest', 'member')                     AS account_type,
    if(u.banned IS NULL, CAST(NULL AS Nullable(UInt8)),
       toUInt8(NOT u.banned))                               AS is_active,
    toDateTime64(u._airbyte_extracted_at, 3)                AS collected_at,
    toUnixTimestamp64Milli(u._airbyte_extracted_at)         AS _version
FROM {{ source('bronze_youtrack', 'youtrack_user') }} u

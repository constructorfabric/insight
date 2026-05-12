-- depends_on: {{ ref('slack__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['slack', 'silver:class_collab_chat_activity']
) }}

-- Slack daily chat activity per user, sourced from admin.analytics.getFile?type=member.
-- Bronze row is already one per (user, date); we simply reshape to the shared
-- class_collab_chat_activity schema. Fields that require raw message-level
-- partitioning (direct vs group vs channel breakdown, reply counts, urgency)
-- are not available from the analytics endpoint and are set to NULL.

SELECT
    u.tenant_id,
    u.source_id AS insight_source_id,
    MD5(concat(
        u.tenant_id, '-',
        u.source_id, '-',
        coalesce(u.user_id, ''), '-',
        toString(toDate(parseDateTimeBestEffort(u.date)))
    )) AS unique_key,
    u.user_id,
    coalesce(u.email_address, '') AS user_name,
    coalesce(u.email_address, '') AS email,
    if(coalesce(u.email_address, '') != '',
       lower(u.email_address),
       lower(u.user_id)) AS person_key,
    toDate(parseDateTimeBestEffort(u.date)) AS date,
    CAST(NULL AS Nullable(Int64)) AS direct_messages,
    CAST(NULL AS Nullable(Int64)) AS group_chat_messages,
    coalesce(u.messages_posted_count, 0) AS total_chat_messages,
    u.channel_messages_posted_count AS channel_posts,
    CAST(NULL AS Nullable(Int64)) AS channel_replies,
    CAST(NULL AS Nullable(Int64)) AS urgent_messages,
    CAST(NULL AS Nullable(String)) AS report_period,
    now() AS collected_at,
    'insight_slack' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version
FROM {{ source('bronze_slack', 'users_details') }} AS u
WHERE u.user_id IS NOT NULL
  AND u.user_id != ''
{% if is_incremental() %}
  AND (
    (SELECT max(date) FROM {{ this }}) IS NULL
    OR toDate(parseDateTimeBestEffort(u.date)) > (SELECT max(date) - INTERVAL 3 DAY FROM {{ this }})
  )
{% endif %}

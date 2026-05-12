-- depends_on: {{ ref('m365__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['m365', 'silver:class_collab_chat_activity']
) }}

SELECT
    tenant_id,
    source_id AS insight_source_id,
    MD5(concat(tenant_id, '-', source_id, '-', coalesce(userPrincipalName, ''), '-', toString(reportRefreshDate))) AS unique_key,
    userPrincipalName AS user_id,
    userPrincipalName AS user_name,
    userPrincipalName AS email,
    if(userPrincipalName IS NOT NULL AND userPrincipalName != '',
       lower(userPrincipalName),
       '') AS person_key,
    toDate(reportRefreshDate) AS date,
    privateChatMessageCount AS direct_messages,
    teamChatMessageCount AS group_chat_messages,
    COALESCE(privateChatMessageCount, 0) + COALESCE(teamChatMessageCount, 0) AS total_chat_messages,
    postMessages AS channel_posts,
    replyMessages AS channel_replies,
    urgentMessages AS urgent_messages,
    reportPeriod AS report_period,
    now() AS collected_at,
    'insight_m365' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version
FROM {{ source('bronze_m365', 'teams_activity') }}
WHERE userPrincipalName IS NOT NULL
  AND userPrincipalName != ''
{% if is_incremental() %}
  AND (
    (SELECT max(date) FROM {{ this }}) IS NULL
    OR toDate(reportRefreshDate) > (SELECT max(date) - INTERVAL 3 DAY FROM {{ this }})
  )
{% endif %}

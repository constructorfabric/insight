{{ config(
    materialized='incremental',
    unique_key='unique_key',
    on_schema_change='append_new_columns',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['m365', 'silver:class_collab_chat_activity']
) }}

-- INVARIANT: privateChatMessageCount counts private-chat messages without splitting
-- one-to-one from group chats; teamChatMessageCount is postMessages + replyMessages.

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
    toInt64(privateChatMessageCount) AS direct_messages,
    CAST(NULL AS Nullable(Int64)) AS group_chat_messages,
    toInt64(privateChatMessageCount) AS direct_and_group_messages,
    toInt64(COALESCE(privateChatMessageCount, 0) + COALESCE(teamChatMessageCount, 0)) AS total_chat_messages,
    toInt64(postMessages) AS channel_posts,
    toInt64(replyMessages) AS channel_replies,
    toInt64(urgentMessages) AS urgent_messages,
    reportPeriod AS report_period,
    now() AS collected_at,
    'insight_m365' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version
FROM {{ source('bronze_m365', 'teams_activity') }} FINAL
WHERE userPrincipalName IS NOT NULL
  AND userPrincipalName != ''
  AND {{ m365_day_was_reported('teams_activity') }}
  -- Drop unlicensed users (guests, ex-employees, service accounts). Their Teams
  -- activity inflates team-level collab counters and produces orphan gold rows
  -- with no matching insight.people entry. `isLicensed` = "Selected if the user
  -- is licensed to use Teams" (MS Graph getTeamsUserActivityUserDetail). Only the
  -- teams_activity report exposes this flag; the email/onedrive/sharepoint feeders
  -- fall back to `assignedProducts`. A NULL flag is treated as unlicensed and
  -- dropped too, per the #736 proposal. See #736.
  AND coalesce(isLicensed, false) = true
{% if is_incremental() %}
  -- Watermark on the source EXTRACT time, not the business date (see zoom model header
  -- for the backfill-strand failure mode this fixes). Re-pulled rows carry a fresh
  -- `_airbyte_extracted_at`, so reprocess every business date touched by a recent extract.
  AND (
    (SELECT count() FROM {{ this }}) = 0
    OR toDate(reportRefreshDate) IN (
      SELECT DISTINCT toDate(reportRefreshDate)
      FROM {{ source('bronze_m365', 'teams_activity') }}
      WHERE _airbyte_extracted_at
            > (SELECT max(_airbyte_extracted_at) FROM {{ source('bronze_m365', 'teams_activity') }}) - INTERVAL 3 DAY
    )
  )
{% endif %}
